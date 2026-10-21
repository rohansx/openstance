//! MCP tool tests: an in-process rmcp client talks to the server over an
//! in-memory duplex pipe (testing-strategy.md, interface tier). The graph is
//! the core test kit (fictional people, `NOW` = 2026-10-20 12:00 UTC); no
//! model and no network are involved.

use std::path::Path;

use openstance_core::search::{self, testkit, SearchRequest};
use openstance_core::{Config, FixedClock, Graph, Providers, QueryPatch};
use openstance_mcp::OpenstanceMcp;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

/// A temp graph.db with the test kit seeded and indexed.
fn seeded(dir: &Path) -> Graph {
    let g = Graph::open(
        Config::default(),
        &dir.join("graph.db"),
        Providers::null(),
        Box::new(FixedClock(testkit::NOW)),
    )
    .unwrap();
    testkit::seed(g.conn());
    search::fts::reindex_all(g.conn()).unwrap();
    g
}

async fn connect(g: Graph) -> RunningService<RoleClient, ()> {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let running = OpenstanceMcp::new(g).serve(server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    ().serve(client_io).await.unwrap()
}

async fn call(client: &RunningService<RoleClient, ()>, name: &str, args: Value) -> CallToolResult {
    let Value::Object(map) = args else {
        panic!("arguments must be an object")
    };
    client
        .peer()
        .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(map))
        .await
        .unwrap()
}

fn structured(r: &CallToolResult) -> Value {
    r.structured_content
        .clone()
        .unwrap_or_else(|| panic!("no structuredContent: {r:?}"))
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_error(r: &CallToolResult) -> bool {
    r.is_error == Some(true)
}

fn error_code(r: &CallToolResult) -> String {
    assert!(is_error(r), "expected an error result: {r:?}");
    structured(r)["error"]["code"].as_str().unwrap().to_string()
}

/// Drop the wall-clock fields so two runs compare equal.
fn redact(mut v: Value) -> Value {
    if let Some(m) = v.as_object_mut() {
        m.remove("took_ms");
        m.remove("timings_ms");
    }
    v
}

/// The `name`d tool JSON blocks of interfaces.md section 2.2.
fn doc_tool(name: &str) -> Value {
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/tech-spec/interfaces.md"),
    )
    .unwrap();
    let section = doc
        .split("### 2.2 Input schemas")
        .nth(1)
        .unwrap()
        .split("### 2.3")
        .next()
        .unwrap();
    section
        .split("```json")
        .skip(1)
        .filter_map(|b| b.split("```").next())
        .filter_map(|b| serde_json::from_str::<Value>(b).ok())
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("{name} not in interfaces.md 2.2"))
}

fn tool_json(t: &Tool) -> Value {
    serde_json::to_value(t).unwrap()
}

// --- P2-MCP-01: registration ----------------------------------------------

#[tokio::test]
async fn lists_exactly_the_phase_2_tools() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["search_people", "get_person", "how_connected"]);
    for t in &tools {
        let a = t.annotations.as_ref().expect("annotations");
        assert_eq!(a.read_only_hint, Some(true), "{} is readOnly", t.name);
        assert_eq!(a.open_world_hint, Some(false), "{} is local", t.name);
        assert!(t.description.as_deref().is_some_and(|d| !d.is_empty()));
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn input_schemas_are_the_ones_in_interfaces_md() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    for t in client.peer().list_all_tools().await.unwrap() {
        let doc = doc_tool(&t.name);
        let got = tool_json(&t);
        assert_eq!(got["inputSchema"], doc["inputSchema"], "{} schema", t.name);
        if let Some(d) = doc.get("description") {
            assert_eq!(&got["description"], d, "{} description", t.name);
        }
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tools_list_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let tools: Vec<Value> = client
        .peer()
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(tool_json)
        .collect();
    insta::assert_json_snapshot!("tools_list", tools);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn server_info_names_openstance_and_offers_tools_only() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.name, "openstance");
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.resources.is_none());
    assert!(info.capabilities.prompts.is_none());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("path_to"))
        .await;
    assert!(r.is_err(), "{r:?}");
    client.cancel().await.unwrap();
}

// --- P2-MCP-02: search_people ---------------------------------------------

/// What the CLI builds for `openstance search <text> --limit <n> --now NOW`.
fn core_search(g: &Graph, text: &str, limit: usize) -> Value {
    let today = search::today_start(testkit::NOW, search::timezone(g.config()));
    let resp = g
        .search(&SearchRequest {
            text: Some(text.into()),
            filters: QueryPatch {
                limit: Some(limit),
                ..QueryPatch::default()
            },
            no_parse: false,
            explain: false,
            today,
            now: testkit::NOW,
        })
        .unwrap();
    serde_json::to_value(resp).unwrap()
}

#[tokio::test]
async fn search_people_returns_the_core_search_response() {
    let dir = tempfile::tempdir().unwrap();
    let expected = redact(core_search(&seeded(dir.path()), "devtools dinner", 5));
    let other = tempfile::tempdir().unwrap();
    let client = connect(seeded(other.path())).await;
    let r = call(
        &client,
        "search_people",
        json!({"text": "devtools dinner", "limit": 5}),
    )
    .await;
    assert!(!is_error(&r), "{r:?}");
    let got = structured(&r);
    assert_eq!(redact(got.clone()), expected);
    assert_eq!(got["v"], 1);
    assert!(got["query"].is_object(), "parsed query is echoed");
    assert!(!got["results"].as_array().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_people_text_summary_names_results_and_quotes_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(&client, "search_people", json!({"text": "devtools dinner"})).await;
    let s = structured(&r);
    let t = text(&r);
    let first = &s["results"][0];
    let name = first["person"]["display_name"].as_str().unwrap();
    assert!(t.contains(name), "{t}");
    if let Some(line) = first["evidence"][0]["line"].as_str() {
        assert!(t.contains(line), "evidence line verbatim: {t}");
    }
    assert!(t.contains(s["rerun"].as_str().unwrap()), "{t}");
    assert!(t.len() < 4000, "short summary");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_people_default_limit_comes_from_mcp_config() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(&client, "search_people", json!({"text": "engineer"})).await;
    assert_eq!(structured(&r)["query"]["limit"], 20);
    let r = call(&client, "search_people", json!({"company": ["Contoso"]})).await;
    assert_eq!(structured(&r)["query"]["limit"], 20);
    assert_eq!(structured(&r)["query"]["company"], json!(["Contoso"]));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_people_maps_every_filter() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(
        &client,
        "search_people",
        json!({
            "company": ["Northwind"], "title": ["founder"], "location": ["San Francisco"],
            "industry": ["fintech"], "tags": ["investor-intro"],
            "since": "2025", "until": "2026-10-01", "last_contact_before": "2026-10-19",
            "met_in_person": true, "replied": true, "min_score": 0.1,
            "lost_touch": false, "include_bulk": true, "limit": 7, "parse": false,
            "explain": true
        }),
    )
    .await;
    assert!(!is_error(&r), "{r:?}");
    let q = &structured(&r)["query"];
    assert_eq!(q["company"], json!(["Northwind"]));
    assert_eq!(q["title"], json!(["founder"]));
    assert_eq!(q["location"], json!(["San Francisco"]));
    assert_eq!(q["industry"], json!(["fintech"]));
    assert_eq!(q["tags"], json!(["investor-intro"]));
    assert!(q["since"].is_string() || q["since"].is_number(), "{q}");
    assert!(q["until"].is_string() || q["until"].is_number(), "{q}");
    assert!(!q["last_contact_before"].is_null(), "{q}");
    assert_eq!(q["met_in_person"], true);
    assert_eq!(q["replied"], true);
    assert_eq!(q["lost_touch"], false);
    assert_eq!(q["include_bulk"], true);
    assert_eq!(q["limit"], 7);
    assert!(
        !structured(&r)["timings_ms"].is_null(),
        "explain fills timings"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_people_parse_false_takes_text_literally() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let parsed = call(
        &client,
        "search_people",
        json!({"text": "founders I met in person"}),
    )
    .await;
    assert_eq!(structured(&parsed)["query"]["met_in_person"], true);
    let literal = call(
        &client,
        "search_people",
        json!({"text": "founders I met in person", "parse": false}),
    )
    .await;
    let q = &structured(&literal)["query"];
    assert_eq!(q["text"], "founders I met in person");
    assert!(q["met_in_person"].is_null(), "{q}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_people_rejects_bad_input_as_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    for args in [
        json!({"bogus": 1}),
        json!({"text": 5}),
        json!({"since": "whenever"}),
        json!({"limit": 0}),
        json!({"limit": 101}),
        json!({"min_score": 1.5}),
        json!({"company": "Contoso"}),
    ] {
        let r = call(&client, "search_people", args.clone()).await;
        assert_eq!(error_code(&r), "bad_request", "{args}");
        assert!(!text(&r).is_empty());
    }
    client.cancel().await.unwrap();
}

// --- P2-MCP-03: get_person ------------------------------------------------

#[tokio::test]
async fn get_person_returns_person_detail_by_id_email_and_name() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let by_id = structured(&call(&client, "get_person", json!({"person": "2"})).await);
    assert_eq!(by_id["v"], 1);
    assert_eq!(by_id["person"]["id"], 2);
    assert_eq!(by_id["person"]["display_name"], "Priya Raman");
    assert!(by_id["identities"].is_array());
    assert!(by_id["recent_interactions"].is_array());
    assert!(by_id["mutuals"].is_array());
    let by_name = structured(&call(&client, "get_person", json!({"person": "Priya Raman"})).await);
    assert_eq!(by_name["person"]["id"], 2);
    let email = by_id["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "email")
        .map(|i| i["value"].as_str().unwrap().to_string())
        .expect("an email identity");
    let by_email = structured(&call(&client, "get_person", json!({"person": email})).await);
    assert_eq!(by_email["person"]["id"], 2);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn get_person_caps_interactions_and_mutuals() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(
        &client,
        "get_person",
        json!({"person": "2", "interactions": 1, "mutuals": 0}),
    )
    .await;
    let d = structured(&r);
    assert!(d["recent_interactions"].as_array().unwrap().len() <= 1);
    assert!(d["mutuals"].as_array().unwrap().is_empty());
    assert!(text(&r).contains("Priya Raman"));
    for bad in [
        json!({}),
        json!({"person": "2", "interactions": 51}),
        json!({"person": "2", "extra": true}),
    ] {
        let r = call(&client, "get_person", bad.clone()).await;
        assert_eq!(error_code(&r), "bad_request", "{bad}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn get_person_ambiguous_name_lists_candidates_instead_of_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let g = seeded(dir.path());
    g.conn()
        .execute(
            "UPDATE person SET display_name = 'Sam Ito' WHERE id = 7",
            [],
        )
        .unwrap();
    let client = connect(g).await;
    let r = call(&client, "get_person", json!({"person": "Sam Ito"})).await;
    assert_eq!(error_code(&r), "ambiguous");
    let s = structured(&r);
    let ids: Vec<i64> = s["error"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [5, 7]);
    let t = text(&r);
    assert!(t.contains("get_person") || t.contains("id"), "{t}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn get_person_unknown_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(&client, "get_person", json!({"person": "Nobody Here"})).await;
    assert_eq!(error_code(&r), "not_found");
    client.cancel().await.unwrap();
}

// --- P2-MCP-04: how_connected ---------------------------------------------

#[tokio::test]
async fn how_connected_returns_the_core_shape() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(&client, "how_connected", json!({"target": "Sam Ito"})).await;
    assert!(!is_error(&r), "{r:?}");
    let h = structured(&r);
    assert_eq!(h["v"], 1);
    assert_eq!(h["target"]["id"], 5);
    let paths = h["paths"].as_array().unwrap();
    assert!(paths.len() <= 3);
    for p in paths {
        assert!(p["via"]["id"].is_number());
        assert!(p["strength"].is_number());
        assert!(p["evidence"].is_array());
    }
    assert!(text(&r).contains("Sam Ito"), "{}", text(&r));
    let one = call(
        &client,
        "how_connected",
        json!({"target": "Sam Ito", "limit": 1}),
    )
    .await;
    assert!(structured(&one)["paths"].as_array().unwrap().len() <= 1);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn how_connected_outside_the_graph_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let r = call(
        &client,
        "how_connected",
        json!({"target": "stranger@nowhere.example"}),
    )
    .await;
    assert!(!is_error(&r), "{r:?}");
    let s = structured(&r);
    assert_eq!(s["found"], false);
    assert!(s["hint"].is_string());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn how_connected_validates_input() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    for bad in [
        json!({}),
        json!({"target": "Sam Ito", "limit": 0}),
        json!({"target": "Sam Ito", "limit": 11}),
        json!({"target": "Sam Ito", "allow_egress": true}),
    ] {
        let r = call(&client, "how_connected", bad.clone()).await;
        assert_eq!(error_code(&r), "bad_request", "{bad}");
    }
    client.cancel().await.unwrap();
}

// --- robustness -----------------------------------------------------------

#[tokio::test]
async fn concurrent_calls_all_complete() {
    let dir = tempfile::tempdir().unwrap();
    let client = connect(seeded(dir.path())).await;
    let peer = client.peer().clone();
    let mut handles = Vec::new();
    for i in 0..8 {
        let peer = peer.clone();
        handles.push(tokio::spawn(async move {
            let args = json!({"text": if i % 2 == 0 { "engineer" } else { "devtools" }});
            let Value::Object(map) = args else {
                unreachable!()
            };
            peer.call_tool(CallToolRequestParams::new("search_people").with_arguments(map))
                .await
                .unwrap()
        }));
    }
    for h in handles {
        let r = h.await.unwrap();
        assert!(!is_error(&r), "{r:?}");
    }
    client.cancel().await.unwrap();
}
