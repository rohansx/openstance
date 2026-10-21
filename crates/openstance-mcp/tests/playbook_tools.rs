//! Phase 3 MCP tools (P3-MCP-01..04): `path_to`, `brief`, `quote_enrich`,
//! `list_views`, `search_view`. An in-process rmcp client talks to the
//! server over a duplex pipe; egress goes to a recording `MockEnricher`, so
//! nothing leaves the machine. The quote-then-confirm flow an agent follows
//! (playbooks/path.md) is exercised end to end.

use std::path::Path;
use std::sync::Arc;

use openstance_core::enrich::{
    EnrichData, EnrichError, EnrichJob, EnrichRequest, EnrichResult, Enricher, ProfileData,
    QuoteLine,
};
use openstance_core::search::{self, testkit};
use openstance_core::view::{CreateView, Ttl, ViewOwner};
use openstance_core::{keys, Config, FixedClock, Graph, Providers, Query};
use openstance_enrich::mock::MockEnricher;
use openstance_mcp::{OpenstanceMcp, ViewsContext};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

/// Shares one recording mock between the server's `Providers` and the test.
struct Shared(Arc<MockEnricher>);

impl Enricher for Shared {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn supports(&self, op: &str) -> bool {
        self.0.supports(op)
    }
    fn quote(&self, jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError> {
        self.0.quote(jobs)
    }
    fn enrich(&self, job: &EnrichJob) -> Result<EnrichResult, EnrichError> {
        self.0.enrich(job)
    }
}

fn dana_profile() -> EnrichRequest {
    EnrichRequest::Profile {
        name: "Dana Whitfield".into(),
        company: Some("Contoso".into()),
        linkedin: None,
    }
}

fn mock() -> Arc<MockEnricher> {
    Arc::new(
        MockEnricher::new()
            .with_fetched_at(testkit::NOW)
            .with_response(
                dana_profile(),
                EnrichData::Profile(ProfileData {
                    display_name: Some("Dana Whitfield".into()),
                    title: Some("CTO".into()),
                    company: Some("Contoso".into()),
                    company_domain: Some("contoso.example".into()),
                    location: None,
                    linkedin: Some("dana-whitfield".into()),
                    past_companies: vec!["Northwind".into()],
                    schools: vec![],
                    headline: None,
                }),
            ),
    )
}

fn graph(dir: &Path, cfg: Config, m: &Arc<MockEnricher>) -> Graph {
    let mut p = Providers::null();
    p.enricher = Box::new(Shared(Arc::clone(m)));
    let g = Graph::open(
        cfg,
        &dir.join("graph.db"),
        p,
        Box::new(FixedClock(testkit::NOW)),
    )
    .unwrap();
    testkit::seed(g.conn());
    search::fts::reindex_all(g.conn()).unwrap();
    g
}

async fn serve(server: OpenstanceMcp) -> RunningService<RoleClient, ()> {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let running = server.serve(server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    ().serve(client_io).await.unwrap()
}

async fn connect(g: Graph) -> RunningService<RoleClient, ()> {
    serve(OpenstanceMcp::new(g)).await
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

fn ok(r: &CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "unexpected error: {r:?}");
    structured(r)
}

fn error_code(r: &CallToolResult) -> String {
    assert_eq!(r.is_error, Some(true), "expected an error result: {r:?}");
    structured(r)["error"]["code"].as_str().unwrap().to_string()
}

fn log_rows(g: &Graph) -> Vec<(String, String)> {
    g.conn()
        .prepare("SELECT op, status FROM enrich_log ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

// --- registration ----------------------------------------------------------

#[tokio::test]
async fn lists_the_eight_phase_3_tools_with_honest_annotations() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "search_people",
            "get_person",
            "how_connected",
            "path_to",
            "brief",
            "list_views",
            "search_view",
            "quote_enrich"
        ]
    );
    for t in &tools {
        let a = t.annotations.as_ref().expect("annotations");
        let spends = matches!(t.name.as_ref(), "path_to" | "brief");
        assert_eq!(a.read_only_hint, Some(!spends), "{} readOnly", t.name);
        let world = matches!(t.name.as_ref(), "path_to" | "brief" | "quote_enrich");
        assert_eq!(a.open_world_hint, Some(world), "{} openWorld", t.name);
        if spends {
            assert_eq!(
                a.destructive_hint,
                Some(false),
                "{} not destructive",
                t.name
            );
        }
    }
    client.cancel().await.unwrap();
}

// --- P3-MCP-01 path_to -----------------------------------------------------

#[tokio::test]
async fn path_to_default_returns_the_quote_and_never_spends() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let g = graph(dir.path(), Config::default(), &m);
    let client = connect(g).await;
    let r = call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "find_email": true }),
    )
    .await;
    let v = ok(&r);
    assert_eq!(v["v"], 1);
    assert_eq!(v["playbook"], "path");
    assert_eq!(v["status"], "quote");
    assert_eq!(v["plan"]["quote"]["backend"], "mock");
    let lines = v["plan"]["quote"]["lines"].as_array().unwrap();
    let ops: Vec<&str> = lines
        .iter()
        .map(|l| l["job"]["request"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["profile", "find_email", "verify_email"]);
    assert_eq!(v["plan"]["quote"]["total_micro_usd"], 30_000);
    assert_eq!(v["receipt"]["requests_sent"], 0);
    assert_eq!(v["receipt"]["spent_micro_usd"], 0);
    assert!(
        m.calls().is_empty(),
        "no enrich call without allow_egress: {:?}",
        m.calls()
    );
    let t = text(&r);
    assert!(t.contains("$0.0300"), "text shows the price: {t}");
    assert!(t.contains("allow_egress"), "text says how to approve: {t}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_quote_then_confirm_spends_exactly_the_quoted_calls() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let args = json!({ "target": "Dana Whitfield", "company": "Contoso" });
    let quote = ok(&call(&client, "path_to", args.clone()).await);
    assert_eq!(quote["status"], "quote");
    assert!(m.calls().is_empty());
    let quoted = quote["plan"]["quote"]["total_micro_usd"].as_u64().unwrap();

    let mut confirmed = args;
    confirmed["allow_egress"] = json!(true);
    let done = ok(&call(&client, "path_to", confirmed).await);
    assert_eq!(done["status"], "done", "{done:#}");
    let ops: Vec<&str> = m.calls().iter().map(|j| j.request.op()).collect();
    assert_eq!(ops, ["profile"]);
    assert_eq!(done["receipt"]["requests_sent"], 1);
    assert!(done["receipt"]["spent_micro_usd"].as_u64().unwrap() <= quoted);
    let paths = done["result"]["paths"].as_array().unwrap();
    assert!(
        paths
            .iter()
            .any(|p| p["via"]["display_name"] == "Marcus Lee"),
        "Contoso colleague found: {done:#}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_max_usd_can_only_lower_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let high = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "max_usd": 5.0 }),
    )
    .await);
    assert_eq!(high["plan"]["quote"]["budget_micro_usd"], 500_000);
    let low = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "max_usd": 0.001,
                "allow_egress": true }),
    )
    .await);
    assert_eq!(low["plan"]["quote"]["budget_micro_usd"], 1_000);
    assert!(m.calls().is_empty(), "over budget: nothing sent");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_read_only_server_never_spends_even_when_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let mut cfg = Config::default();
    cfg.mcp.read_only = true;
    let client = connect(graph(dir.path(), cfg, &m)).await;
    let v = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "allow_egress": true }),
    )
    .await);
    assert_eq!(v["status"], "quote");
    assert!(m.calls().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_plan_only_and_in_graph_targets_make_no_calls() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let plan = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "plan_only": true,
                "allow_egress": true }),
    )
    .await);
    assert_eq!(plan["status"], "quote");
    assert!(plan["result"].is_null());
    let known = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Marcus Lee", "allow_egress": true }),
    )
    .await);
    assert_eq!(known["status"], "done");
    assert!(m.calls().is_empty(), "{:?}", m.calls());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_validates_input() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    for args in [
        json!({}),
        json!({ "target": "x", "limit": 0 }),
        json!({ "target": "x", "limit": 11 }),
        json!({ "target": "x", "max_usd": -1.0 }),
        json!({ "target": "x", "surprise": true }),
        json!({ "target": "   " }),
    ] {
        let r = call(&client, "path_to", args.clone()).await;
        assert_eq!(error_code(&r), "bad_request", "{args}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn path_to_matches_the_core_playbook_function() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let g = graph(dir.path(), Config::default(), &m);
    let direct = g
        .run_playbook(
            "path",
            &json!({ "target": "Dana Whitfield", "company": "Contoso", "limit": 3 }),
            &openstance_core::playbook::RunOptions::default(),
        )
        .unwrap();
    let client = connect(g).await;
    let mcp = ok(&call(
        &client,
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso" }),
    )
    .await);
    let mut want = serde_json::to_value(&direct).unwrap();
    let mut got = mcp;
    for v in [&mut want, &mut got] {
        v["plan"]["run_id"] = Value::Null;
        v["receipt"]["run_id"] = Value::Null;
        v["receipt"]["enrich_log_ids"] = Value::Null;
    }
    assert_eq!(got, want);
    client.cancel().await.unwrap();
}

// --- P3-MCP-02 brief -------------------------------------------------------

#[tokio::test]
async fn brief_without_web_is_local_and_returns_markdown_and_facts() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let r = call(&client, "brief", json!({ "person": "Priya Raman" })).await;
    let v = ok(&r);
    assert_eq!(v["playbook"], "brief");
    assert_eq!(v["status"], "done");
    let md = v["result"]["markdown"].as_str().unwrap();
    assert!(md.contains("Priya Raman"), "{md}");
    assert!(!v["result"]["facts"].as_array().unwrap().is_empty());
    assert!(text(&r).contains("Priya Raman"));
    assert!(m.calls().is_empty() && m.quotes().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn brief_web_without_egress_skips_the_search_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let v = ok(&call(
        &client,
        "brief",
        json!({ "person": "Priya Raman", "web": true }),
    )
    .await);
    assert_eq!(v["status"], "quote");
    assert!(
        v["result"]["markdown"].as_str().is_some(),
        "local brief still rendered"
    );
    let notes = v["notes"].to_string();
    assert!(notes.contains("allow_egress"), "{notes}");
    assert!(m.calls().is_empty());
    let step = v["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["egress"] == "web_search")
        .expect("web step");
    assert_eq!(step["will_run"], "skip_no_egress");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn brief_unknown_person_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let r = call(&client, "brief", json!({ "person": "Nobody Atall" })).await;
    assert_eq!(error_code(&r), "not_found");
    let r = call(&client, "brief", json!({ "person": "Priya", "extra": 1 })).await;
    assert_eq!(error_code(&r), "bad_request");
    client.cancel().await.unwrap();
}

// --- P3-MCP-03 quote_enrich -------------------------------------------------

#[tokio::test]
async fn quote_enrich_prices_requests_and_never_sends() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let g = graph(dir.path(), Config::default(), &m);
    let client = connect(g).await;
    let r = call(
        &client,
        "quote_enrich",
        json!({ "requests": [
            { "kind": "profile", "name": "Dana Whitfield", "company": "Contoso" },
            { "kind": "verify_email", "email": "dana@contoso.example" },
            { "kind": "web_search", "query": "Dana Whitfield Contoso" }
        ] }),
    )
    .await;
    let v = ok(&r);
    assert_eq!(v["v"], 1);
    assert_eq!(v["backend"], "mock");
    assert_eq!(v["lines"].as_array().unwrap().len(), 3);
    assert_eq!(v["total_micro_usd"], 30_000);
    assert_eq!(v["budget_micro_usd"], 500_000);
    assert_eq!(v["within_budget"], true);
    assert!(v["quoted_at"].as_str().unwrap().starts_with("2026-10-20T"));
    assert!(m.calls().is_empty(), "quote only");
    assert_eq!(m.quotes().len(), 1);
    assert!(text(&r).contains("$0.0300"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn quote_enrich_fills_names_from_person_id_and_skips_no_enrich_people() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let g = graph(dir.path(), Config::default(), &m);
    g.conn()
        .execute("UPDATE person SET no_enrich = 1 WHERE id = 6", [])
        .unwrap();
    let client = connect(g).await;
    let v = ok(&call(
        &client,
        "quote_enrich",
        json!({ "requests": [
            { "kind": "profile", "person_id": 2 },
            { "kind": "profile", "person_id": 6 }
        ] }),
    )
    .await);
    let lines = v["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "{v:#}");
    assert_eq!(lines[0]["job"]["request"]["name"], "Priya Raman");
    let skipped = v["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["person_id"], 6);
    assert_eq!(skipped[0]["reason"], "no_enrich");
    for q in m.quotes() {
        assert!(
            q.iter().all(|j| j.person_id != Some(6)),
            "no_enrich never priced"
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn quote_enrich_logs_only_quoted_rows() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let g = graph(dir.path(), Config::default(), &m);
    let db = dir.path().join("graph.db");
    let client = connect(g).await;
    ok(&call(
        &client,
        "quote_enrich",
        json!({ "requests": [ { "kind": "company", "domain": "contoso.example" } ] }),
    )
    .await);
    client.cancel().await.unwrap();
    let check = Graph::open(
        Config::default(),
        &db,
        Providers::null(),
        Box::new(FixedClock(testkit::NOW)),
    )
    .unwrap();
    assert_eq!(log_rows(&check), [("company".into(), "quoted".into())]);
}

#[tokio::test]
async fn quote_enrich_rejects_bad_requests() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    for args in [
        json!({}),
        json!({ "requests": [] }),
        json!({ "requests": [ { "kind": "teleport" } ] }),
        json!({ "requests": [ { "kind": "verify_email" } ] }),
        json!({ "requests": [ { "kind": "find_email", "name": "Dana" } ] }),
        json!({ "requests": [ { "kind": "profile" } ] }),
        json!({ "requests": [ { "kind": "profile", "person_id": 999 } ] }),
    ] {
        let r = call(&client, "quote_enrich", args.clone()).await;
        let code = error_code(&r);
        assert!(
            code == "bad_request" || code == "not_found",
            "{args}: {code}"
        );
    }
    assert!(m.calls().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn quote_enrich_with_backend_none_quotes_zero() {
    let dir = tempfile::tempdir().unwrap();
    let g = Graph::open(
        Config::default(),
        &dir.path().join("graph.db"),
        Providers::null(),
        Box::new(FixedClock(testkit::NOW)),
    )
    .unwrap();
    let client = connect(g).await;
    let v = ok(&call(
        &client,
        "quote_enrich",
        json!({ "requests": [ { "kind": "verify_email", "email": "a@b.example" } ] }),
    )
    .await);
    assert_eq!(v["backend"], "none");
    assert_eq!(v["total_micro_usd"], 0);
    assert_eq!(v["skipped"][0]["reason"], "backend_none");
    client.cancel().await.unwrap();
}

// --- P3-MCP-04 list_views / search_view --------------------------------------

/// A graph with one own view, plus a views dir holding an import of it.
fn with_views(dir: &Path, m: &Arc<MockEnricher>) -> OpenstanceMcp {
    let mut g = graph(dir, Config::default(), m);
    let key = keys::ensure(&dir.join("age.key"), testkit::NOW).unwrap();
    let owner_pub = key.public();
    let query: Query = serde_json::from_value(json!({ "company": ["Northwind"] })).unwrap();
    g.view_create(&CreateView {
        name: "northwind".into(),
        query,
        ttl: Ttl::Never,
        recipients: vec![],
        replace: false,
    })
    .unwrap();
    let exported = g
        .view_export(
            "northwind",
            &openstance_core::view::ost::ExportOptions {
                owner_pubkey: &owner_pub,
                owner_label: "owen",
                armor: false,
                tmp_parent: Some(dir),
                version: "test",
            },
        )
        .unwrap();
    let views = dir.join("views");
    openstance_core::view::inbox::import(
        &views,
        &key.identity,
        &exported.bytes,
        1 << 20,
        testkit::NOW,
    )
    .unwrap();
    OpenstanceMcp::new(g).with_views(ViewsContext {
        dir: views,
        owner: ViewOwner {
            pubkey: Some(owner_pub),
            label: Some("owen".into()),
        },
    })
}

#[tokio::test]
async fn list_views_filters_by_direction() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = serve(with_views(dir.path(), &m)).await;
    let all = ok(&call(&client, "list_views", json!({})).await);
    let dirs: Vec<&str> = all["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["direction"].as_str().unwrap())
        .collect();
    assert_eq!(dirs, ["own", "imported"]);
    let own = ok(&call(&client, "list_views", json!({ "direction": "own" })).await);
    assert_eq!(own["views"].as_array().unwrap().len(), 1);
    assert_eq!(own["views"][0]["name"], "northwind");
    assert_eq!(own["views"][0]["owner"]["label"], "owen");
    let imported = ok(&call(&client, "list_views", json!({ "direction": "imported" })).await);
    assert_eq!(imported["views"].as_array().unwrap().len(), 1);
    assert_eq!(imported["views"][0]["direction"], "imported");
    let bad = call(&client, "list_views", json!({ "direction": "sideways" })).await;
    assert_eq!(error_code(&bad), "bad_request");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn search_view_searches_imported_views() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = serve(with_views(dir.path(), &m)).await;
    let r = call(
        &client,
        "search_view",
        json!({ "view": "*", "text": "Priya" }),
    )
    .await;
    let v = ok(&r);
    assert_eq!(v["v"], 1);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{v:#}");
    assert_eq!(results[0]["person"]["display_name"], "Priya Raman");
    assert_eq!(results[0]["view"]["name"], "northwind");
    assert!(text(&r).contains("Priya Raman"));
    let named = ok(&call(
        &client,
        "search_view",
        json!({ "view": "northwind", "company": ["northwind"], "limit": 5 }),
    )
    .await);
    assert!(!named["results"].as_array().unwrap().is_empty());
    let missing = call(&client, "search_view", json!({ "view": "nope" })).await;
    assert_eq!(error_code(&missing), "not_found");
    let bad = call(&client, "search_view", json!({ "view": "*", "limit": 0 })).await;
    assert_eq!(error_code(&bad), "bad_request");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn view_tools_without_a_views_dir_list_own_views_only() {
    let dir = tempfile::tempdir().unwrap();
    let m = mock();
    let client = connect(graph(dir.path(), Config::default(), &m)).await;
    let v = ok(&call(&client, "list_views", json!({})).await);
    assert_eq!(v["views"], json!([]));
    let r = call(&client, "search_view", json!({ "view": "*" })).await;
    assert_eq!(ok(&r)["results"], json!([]));
    client.cancel().await.unwrap();
}
