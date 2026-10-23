//! MCP over streamable HTTP at `/mcp` (P4-API-05): the same 8 tools and
//! schemas as stdio, behind the same bearer token and Host/Origin checks.
//!
//! Results over HTTP may leave the machine (interfaces.md section 2), so
//! every `do_not_share` person is removed from tool results: list entries
//! that are, or go via, such a person are dropped, and a call about one
//! (`get_person`, `how_connected` target) answers `not_found`. Local stdio
//! MCP and the HTTP API used by the user's own browser are unaffected.

use std::collections::HashSet;
use std::sync::{Arc, PoisonError};

use openstance_core::json::V;
use openstance_core::view::ViewOwner;
use openstance_mcp::schema;
use openstance_mcp::tools::ToolOutput;
use openstance_mcp::{text, OpenstanceMcp, ViewsContext};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
    ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::Value;

use crate::pool::SharedGraph;
use crate::server::AppState;

/// The rmcp handler for `/mcp`.
#[derive(Clone)]
pub struct HttpMcp {
    inner: OpenstanceMcp,
    graph: SharedGraph,
}

impl std::fmt::Debug for HttpMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpMcp").finish_non_exhaustive()
    }
}

/// The tower service mounted at `/mcp`. Host and Origin are checked by the
/// router's layers (rmcp's own loopback-only Host list would reject the
/// `host:port` forms the router accepts, so it is turned off here).
pub fn service(state: &Arc<AppState>) -> StreamableHttpService<HttpMcp, LocalSessionManager> {
    let graph = state.pool.writer();
    let views = state.opts.share.as_ref().map(|s| ViewsContext {
        dir: s.views.clone(),
        owner: ViewOwner {
            pubkey: openstance_core::keys::load(&s.key).ok().map(|k| k.public()),
            label: state.opts.owner_label.clone(),
        },
    });
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    StreamableHttpService::new(
        move || {
            let mut inner = OpenstanceMcp::shared(Arc::clone(&graph));
            if let Some(v) = &views {
                inner = inner.with_views(v.clone());
            }
            Ok(HttpMcp {
                inner,
                graph: Arc::clone(&graph),
            })
        },
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

impl HttpMcp {
    async fn hidden_people(&self) -> HashSet<i64> {
        let graph = Arc::clone(&self.graph);
        tokio::task::spawn_blocking(move || {
            let g = graph.lock().unwrap_or_else(PoisonError::into_inner);
            let mut st = match g
                .conn()
                .prepare("SELECT id FROM person WHERE do_not_share = 1")
            {
                Ok(s) => s,
                Err(_) => return HashSet::new(),
            };
            st.query_map([], |r| r.get::<_, i64>(0))
                .map(|rows| rows.filter_map(Result::ok).collect())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }
}

fn to_result(out: ToolOutput) -> CallToolResult {
    let mut r = if out.is_error {
        CallToolResult::structured_error(out.structured)
    } else {
        CallToolResult::structured(out.structured)
    };
    r.content = vec![Content::text(out.text)];
    r
}

impl ServerHandler for HttpMcp {
    fn get_info(&self) -> ServerInfo {
        self.inner.get_info()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(schema::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();
        if !schema::is_tool(&name) {
            return Err(McpError::invalid_params(
                format!("unknown tool {name:?}"),
                None,
            ));
        }
        let out = self.inner.run_tool(name.clone(), request.arguments).await;
        if out.is_error {
            return Ok(to_result(out));
        }
        let hidden = self.hidden_people().await;
        Ok(to_result(redact_output(&name, out, &hidden)))
    }
}

/// Remove `hidden` people from a successful tool output.
pub fn redact_output(tool: &str, out: ToolOutput, hidden: &HashSet<i64>) -> ToolOutput {
    if hidden.is_empty() {
        return out;
    }
    let mut v = out.structured;
    if about_hidden(&v, hidden) {
        return ToolOutput::error(&openstance_core::Error::NotFound(
            "this person is marked do-not-share and is not available over HTTP".into(),
        ));
    }
    redact(&mut v, hidden);
    let text = match tool {
        schema::SEARCH_PEOPLE => text::search(&v),
        schema::GET_PERSON => text::person(&v),
        schema::HOW_CONNECTED => text::how(&v),
        schema::PATH_TO => text::path_to(&v),
        schema::BRIEF => text::brief(&v),
        schema::QUOTE_ENRICH => text::quote(&v),
        schema::LIST_VIEWS => text::views(&v),
        schema::SEARCH_VIEW => text::view_search(&v),
        _ => out.text,
    };
    ToolOutput {
        structured: v,
        text,
        is_error: false,
    }
}

fn id_of(v: &Value) -> Option<i64> {
    v.get("id").and_then(Value::as_i64)
}

fn hidden_ref(v: &Value, hidden: &HashSet<i64>) -> bool {
    v.is_object() && id_of(v).is_some_and(|id| hidden.contains(&id))
}

/// The whole document is about a hidden person (`get_person`, a target).
fn about_hidden(v: &Value, hidden: &HashSet<i64>) -> bool {
    let at = |p: &[&str]| {
        let mut cur = v;
        for k in p {
            match cur.get(*k) {
                Some(x) => cur = x,
                None => return false,
            }
        }
        hidden_ref(cur, hidden)
    };
    at(&["person"]) || at(&["target"]) || at(&["result", "person"]) || at(&["result", "target"])
}

/// An array element that is, or goes via, a hidden person.
fn hidden_element(e: &Value, hidden: &HashSet<i64>) -> bool {
    if e.get("display_name").is_some() && hidden_ref(e, hidden) {
        return true;
    }
    ["person", "via", "with"]
        .iter()
        .any(|k| e.get(*k).is_some_and(|x| hidden_ref(x, hidden)))
}

fn redact(v: &mut Value, hidden: &HashSet<i64>) {
    match v {
        Value::Array(items) => {
            items.retain(|e| !hidden_element(e, hidden));
            for e in items.iter_mut() {
                redact(e, hidden);
            }
        }
        Value::Object(map) => {
            let before = map.get("results").and_then(Value::as_array).map(Vec::len);
            for (_, x) in map.iter_mut() {
                redact(x, hidden);
            }
            // search_people: keep `total` and `rank` consistent.
            if let (Some(before), Some(after)) = (
                before,
                map.get("results").and_then(Value::as_array).map(Vec::len),
            ) {
                if map.get("v") == Some(&Value::from(V)) && map.contains_key("total") {
                    let removed = (before - after) as u64;
                    if let Some(t) = map.get("total").and_then(Value::as_u64) {
                        map.insert("total".into(), Value::from(t.saturating_sub(removed)));
                    }
                    if let Some(Value::Array(rs)) = map.get_mut("results") {
                        for (i, r) in rs.iter_mut().enumerate() {
                            if r.get("rank").is_some() {
                                r["rank"] = Value::from(i + 1);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hid() -> HashSet<i64> {
        [2].into_iter().collect()
    }

    #[test]
    fn search_results_drop_hidden_people_and_rerank() {
        let out = ToolOutput {
            structured: json!({ "v": 1, "total": 3, "results": [
                { "rank": 1, "person": { "id": 2, "display_name": "A" } },
                { "rank": 2, "person": { "id": 3, "display_name": "B" } },
                { "rank": 3, "person": { "id": 4, "display_name": "C" } }
            ]}),
            text: String::new(),
            is_error: false,
        };
        let r = redact_output(schema::SEARCH_PEOPLE, out, &hid());
        assert_eq!(r.structured["total"], 2);
        assert_eq!(r.structured["results"][0]["person"]["id"], 3);
        assert_eq!(r.structured["results"][0]["rank"], 1);
        assert_eq!(r.structured["results"][1]["rank"], 2);
    }

    #[test]
    fn a_document_about_a_hidden_person_becomes_not_found() {
        let out = ToolOutput {
            structured: json!({ "v": 1, "person": { "id": 2, "display_name": "A" } }),
            text: "A".into(),
            is_error: false,
        };
        let r = redact_output(schema::GET_PERSON, out, &hid());
        assert!(r.is_error);
        assert_eq!(r.structured["error"]["code"], "not_found");
        assert!(!r.text.contains('A') || r.text.contains("not"));
    }

    #[test]
    fn paths_mutuals_and_evidence_via_hidden_people_are_dropped() {
        let out = ToolOutput {
            structured: json!({ "v": 1, "target": { "id": 9, "display_name": "T" },
              "paths": [ { "via": { "id": 2, "display_name": "A" } }, { "via": { "id": 5 } } ],
              "mutuals": [ { "person": { "id": 2, "display_name": "A" } } ],
              "evidence": [ { "kind": "tie", "with": { "id": 2, "display_name": "A" } } ] }),
            text: String::new(),
            is_error: false,
        };
        let r = redact_output(schema::HOW_CONNECTED, out, &hid());
        assert_eq!(r.structured["paths"].as_array().unwrap().len(), 1);
        assert!(r.structured["mutuals"].as_array().unwrap().is_empty());
        assert!(r.structured["evidence"].as_array().unwrap().is_empty());
    }

    #[test]
    fn nothing_hidden_is_a_no_op() {
        let out = ToolOutput {
            structured: json!({ "v": 1, "person": { "id": 2 } }),
            text: "x".into(),
            is_error: false,
        };
        let r = redact_output(schema::GET_PERSON, out.clone(), &HashSet::new());
        assert_eq!(r, out);
    }
}
