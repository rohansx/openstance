//! `openstance mcp` with the Phase 3 tools, end to end through the real
//! binary (P3-MCP-01..04): the eight tools are listed, the enricher comes
//! from `[enrich]` in the config (here `backend = "treg"` pointed at an
//! in-process HTTP server that answers like a treg router), the default call
//! only quotes, and a paid call that the router refuses (402) is surfaced
//! cleanly with an `error` row in `enrich_log` and nothing written to the
//! graph. Nothing here touches the internet.

#![cfg(feature = "server")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use openstance_core::search::testkit;
use serde_json::{json, Value};

const NOW: &str = "2026-10-20T12:00:00Z";
const TOKEN: &str = "treg_SENTINEL_MCP_TOKEN_5d2e";

fn env(c: &mut Command, home: &Path) {
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OST_TEST_TREG_TOKEN", TOKEN)
        .env("NO_COLOR", "1");
}

fn cli(home: &Path, args: &[&str]) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
    env(&mut c, home);
    c.env_remove("OPENSTANCE_LOG");
    let out = c.args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn db(home: &Path) -> rusqlite::Connection {
    openstance_core::db::open(
        &home.join("graph.db"),
        &openstance_core::db::DbOptions {
            busy_timeout_ms: 5000,
        },
    )
    .unwrap()
}

fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cli(dir.path(), &["init", "--me", "owner@example.com"]);
    let conn = db(dir.path());
    testkit::seed(&conn);
    openstance_core::search::fts::reindex_all(&conn).unwrap();
    dir
}

// ------------------------------------------------------------ treg-shaped HTTP

type Log = Arc<Mutex<Vec<(String, String, String)>>>;

/// Prices from the open catalog; every paid call is refused with 402.
fn respond(method: &str, path: &str) -> (u16, String) {
    match (method, path) {
        ("GET", p) if p.starts_with("/catalog/endpoints/") => (
            200,
            json!({ "routing": { "plan": [ { "endpoint_id": "x", "usd": 0.01 } ] } }).to_string(),
        ),
        ("POST", p) if p.starts_with("/call/") => (
            402,
            json!({ "detail": { "error": "insufficient_balance", "balance_micro": 0,
                                "estimated_cost_micro": 10000 } })
            .to_string(),
        ),
        _ => (404, json!({ "detail": "no such tool" }).to_string()),
    }
}

fn start_router() -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let log = log2.clone();
            std::thread::spawn(move || handle(stream, &log));
        }
    });
    (base, log)
}

fn handle(stream: TcpStream, log: &Mutex<Vec<(String, String, String)>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    let token = headers.get("x-treg-token").cloned().unwrap_or_default();
    log.lock()
        .unwrap()
        .push((method.clone(), path.clone(), token));
    let (status, body) = respond(&method, &path);
    let mut out = stream;
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(body.as_bytes());
}

fn use_treg(home: &Path, base: &str) {
    std::fs::write(
        home.join("config.toml"),
        format!(
            "[me]\nidentities = [\"email:owner@example.com\"]\n\n[enrich]\nbackend = \"treg\"\n\n[enrich.treg]\nendpoint = \"{base}\"\ntoken_env = \"OST_TEST_TREG_TOKEN\"\n"
        ),
    )
    .unwrap();
}

// ------------------------------------------------------------ MCP client

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Mcp {
    fn start(home: &Path) -> Mcp {
        let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
        env(&mut c, home);
        c.env("OPENSTANCE_LOG", "debug")
            .args(["mcp", "--now", NOW])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Mcp {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = s.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "openstance-test", "version": "0" } }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "openstance", "{init}");
        s.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        s
    }

    fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server closed"
            );
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("non-JSON on stdout ({e}): {line:?}"));
            if v["id"] == id {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": args}))["result"].clone()
    }

    /// Close stdin, wait; returns stderr (the log).
    fn finish(mut self) -> String {
        drop(self.stdin.take());
        let out = self.child.wait_with_output().unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
}

// ------------------------------------------------------------ tests

#[test]
fn lists_eight_tools_and_the_default_config_never_reaches_the_network() {
    let home = seeded();
    let mut s = Mcp::start(home.path());
    let tools = s.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
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
    // backend = "none": even allow_egress=true stays local.
    let r = s.call(
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "allow_egress": true }),
    );
    assert_eq!(r["isError"], false, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["plan"]["quote"]["backend"], "none");
    assert_eq!(v["receipt"]["requests_sent"], 0);
    let q = s.call(
        "quote_enrich",
        json!({ "requests": [ { "kind": "verify_email", "email": "a@b.example" } ] }),
    );
    assert_eq!(q["structuredContent"]["total_micro_usd"], 0, "{q}");
    s.finish();
    let n: i64 = db(home.path())
        .query_row("SELECT count(*) FROM enrich_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "backend none writes no enrich_log rows");
}

#[test]
fn treg_backend_from_config_quotes_without_the_token_or_contact_data() {
    let home = seeded();
    let (base, log) = start_router();
    use_treg(home.path(), &base);
    let mut s = Mcp::start(home.path());
    let r = s.call(
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso" }),
    );
    assert_eq!(r["isError"], false, "{r}");
    let v = &r["structuredContent"];
    assert_eq!(v["status"], "quote", "{v}");
    assert_eq!(v["plan"]["quote"]["backend"], "treg");
    // A profile by company name is two routed calls (company, then person).
    assert_eq!(v["plan"]["quote"]["total_micro_usd"], 20_000);
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("$0.0200") && text.contains("allow_egress=true"),
        "{text}"
    );

    let q = s.call(
        "quote_enrich",
        json!({ "requests": [
            { "kind": "verify_email", "email": "dana@contoso.example" },
            { "kind": "web_search", "query": "Dana Whitfield" } ] }),
    );
    assert_eq!(q["structuredContent"]["backend"], "treg", "{q}");
    assert_eq!(q["structuredContent"]["total_micro_usd"], 20_000);
    let stderr = s.finish();

    let reqs = log.lock().unwrap().clone();
    assert!(!reqs.is_empty());
    for (method, path, token) in &reqs {
        assert_eq!(method, "GET", "{path}: quoting never calls");
        assert!(path.starts_with("/catalog/endpoints/"), "{path}");
        assert!(token.is_empty(), "no token on catalog reads");
        assert!(
            !path.contains("Dana") && !path.contains("contoso"),
            "{path}"
        );
    }
    assert!(!stderr.contains(TOKEN), "token never logged");
}

#[test]
fn a_refused_paid_call_is_reported_logged_as_error_and_writes_nothing() {
    let home = seeded();
    let (base, log) = start_router();
    use_treg(home.path(), &base);
    let people_before: i64 = db(home.path())
        .query_row("SELECT count(*) FROM person", [], |r| r.get(0))
        .unwrap();
    let mut s = Mcp::start(home.path());
    let r = s.call(
        "path_to",
        json!({ "target": "Dana Whitfield", "company": "Contoso", "allow_egress": true }),
    );
    assert_eq!(
        r["isError"], false,
        "a refused call is an outcome, not a crash: {r}"
    );
    let v = &r["structuredContent"];
    assert_eq!(v["status"], "partial", "{v}");
    assert_eq!(v["receipt"]["spent_micro_usd"], 0);
    let notes = v["notes"].to_string().to_lowercase();
    assert!(
        notes.contains("payment") || notes.contains("402"),
        "{notes}"
    );
    let stderr = s.finish();
    assert!(!stderr.contains(TOKEN));

    let posts: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.0 == "POST")
        .cloned()
        .collect();
    assert!(!posts.is_empty(), "the approved call was attempted");
    assert!(posts.iter().all(|p| p.2 == TOKEN), "calls carry the token");

    let conn = db(home.path());
    let rows: Vec<(String, String, Option<i64>)> = conn
        .prepare("SELECT op, status, cost_micro_usd FROM enrich_log ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "profile");
    assert_eq!(rows[0].1, "error");
    let people_after: i64 = conn
        .query_row("SELECT count(*) FROM person", [], |r| r.get(0))
        .unwrap();
    assert_eq!(people_after, people_before, "no partial writes");
    let enrich_sources: i64 = conn
        .query_row(
            "SELECT count(*) FROM source WHERE kind LIKE 'enrich:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enrich_sources, 0, "nothing committed");
}

#[test]
fn list_views_shows_own_views_with_my_key() {
    let home = seeded();
    let h = home.path();
    let pubkey = cli(h, &["keys", "show"]).trim().to_string();
    assert!(pubkey.starts_with("age1"), "{pubkey}");
    cli(
        h,
        &[
            "view",
            "create",
            "northwind",
            "--query",
            r#"{"company":["Northwind"]}"#,
            "--ttl",
            "30d",
        ],
    );
    let mut s = Mcp::start(h);
    let r = s.call("list_views", json!({ "direction": "own" }));
    let v = &r["structuredContent"];
    assert_eq!(v["views"][0]["name"], "northwind", "{r}");
    assert_eq!(v["views"][0]["owner"]["pubkey"], pubkey.as_str());
    let r = s.call("search_view", json!({ "view": "*" }));
    assert_eq!(r["structuredContent"]["results"], json!([]), "{r}");
    s.finish();
}
