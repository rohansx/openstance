//! P4-API-05: the same 8 MCP tools over streamable HTTP at `/mcp`, behind
//! the bearer token, with `do_not_share` people left out of every result
//! (interfaces.md section 2: results over HTTP may leave the machine).

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use axum::Router;
use common::*;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

struct Client {
    router: Router,
    session: Option<String>,
    next: i64,
}

impl Client {
    async fn start(h: &Harness) -> Client {
        let mut c = Client {
            router: h.router(),
            session: None,
            next: 1,
        };
        let init = c
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "openstance-test", "version": "0" }
                }),
            )
            .await;
        assert_eq!(init["result"]["serverInfo"]["name"], "openstance", "{init}");
        c.notify("notifications/initialized").await;
        c
    }

    fn builder(&self) -> axum::http::request::Builder {
        let mut b = req(Method::POST, "/mcp")
            .header("host", HOST)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18");
        if let Some(s) = &self.session {
            b = b.header("mcp-session-id", s);
        }
        b
    }

    async fn notify(&mut self, method: &str) {
        let msg = json!({ "jsonrpc": "2.0", "method": method });
        let resp = self
            .router
            .clone()
            .oneshot(self.builder().body(Body::from(msg.to_string())).unwrap())
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{}", resp.status());
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let resp = self
            .router
            .clone()
            .oneshot(self.builder().body(Body::from(msg.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        if let Some(s) = resp.headers().get("mcp-session-id") {
            self.session = Some(s.to_str().unwrap().to_string());
        }
        let mut body = resp.into_body();
        let mut buf = String::new();
        let read = async {
            while let Some(frame) = body.frame().await {
                if let Ok(data) = frame.unwrap().into_data() {
                    buf.push_str(&String::from_utf8_lossy(&data));
                }
                for line in buf.lines() {
                    let payload = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
                    if let Ok(v) = serde_json::from_str::<Value>(payload) {
                        if v["id"] == id {
                            return v;
                        }
                    }
                }
            }
            panic!("no response to request {id}: {buf}");
        };
        tokio::time::timeout(Duration::from_secs(20), read)
            .await
            .expect("mcp response in time")
    }

    async fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self
            .request("tools/call", json!({ "name": tool, "arguments": args }))
            .await;
        r["result"].clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lists_the_same_eight_tools_as_stdio() {
    let h = harness();
    let mut c = Client::start(&h).await;
    let r = c.request("tools/list", json!({})).await;
    let tools = r["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 8);
    let want = serde_json::to_value(openstance_mcp::schema::tools()).unwrap();
    assert_eq!(Value::Array(tools), want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_matches_the_tool_output() {
    let h = harness();
    let mut c = Client::start(&h).await;
    let r = c
        .call("search_people", json!({ "text": "northwind", "limit": 5 }))
        .await;
    let mut got = r["structuredContent"].clone();
    let mut want = h
        .core(|g| {
            openstance_mcp::tools::call(
                g,
                "search_people",
                json!({ "text": "northwind", "limit": 5 })
                    .as_object()
                    .cloned(),
            )
            .unwrap()
            .structured
        })
        .await;
    got["took_ms"] = Value::Null;
    want["took_ms"] = Value::Null;
    assert_eq!(got, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn do_not_share_people_never_leave_over_http() {
    let h = harness();
    h.pool
        .write(|g| {
            g.conn()
                .execute_batch(
                    "UPDATE person SET do_not_share = 1 WHERE id = 2;
                     INSERT INTO tie(a, b, kind, weight, last_at, n_shared)
                       VALUES (2, 3, 'email', 0.4, 0, 1);",
                )
                .unwrap();
            Ok(())
        })
        .await
        .unwrap();
    let mut c = Client::start(&h).await;

    let r = c
        .call("search_people", json!({ "text": "northwind" }))
        .await;
    let results = r["structuredContent"]["results"].as_array().unwrap();
    assert!(
        results.iter().all(|x| x["person"]["id"] != 2),
        "{results:?}"
    );
    assert!(!r["content"][0]["text"].as_str().unwrap().contains("Priya"));

    let r = c.call("get_person", json!({ "person": "2" })).await;
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["error"]["code"], "not_found");

    let local = h
        .core(|g| {
            openstance_mcp::tools::call(
                g,
                "how_connected",
                json!({ "target": "3" }).as_object().cloned(),
            )
            .unwrap()
            .structured
        })
        .await;
    assert_eq!(
        local["paths"][0]["via"]["id"], 2,
        "stdio would show the path"
    );
    let r = c.call("how_connected", json!({ "target": "3" })).await;
    let paths = r["structuredContent"]["paths"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(paths.iter().all(|p| p["via"]["id"] != 2), "{paths:?}");

    // The local HTTP API (the user's own browser) still shows them.
    let (_, v) = h.get("/people/2").await;
    assert_eq!(v["person"]["do_not_share"], true);
}
