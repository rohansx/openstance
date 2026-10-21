//! P3-CLI-01 `path`, P3-CLI-02 `brief` and P3-CLI-04 `enrich log`, end to
//! end against a temp `OPENSTANCE_HOME` seeded with the core test kit.
//! Paid steps run against an in-process HTTP server that answers like a
//! treg router (canned bodies written for these tests); nothing here
//! touches the internet.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
use openstance_core::playbook::RunOptions;
use openstance_core::search::testkit;
use openstance_core::{Config, FixedClock, Graph, Providers};
use serde_json::{json, Value};

const NOW: &str = "2026-10-20T12:00:00Z";
const TOKEN: &str = "treg_SENTINEL_CLI_TOKEN_91c0";

fn ost(home: &Path) -> Command {
    let mut c = Command::cargo_bin("openstance").unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OST_TEST_TREG_TOKEN", TOKEN)
        .env("NO_COLOR", "1");
    c
}

fn run(c: &mut Command) -> (i32, String, String) {
    let out = c.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn ok_json(c: &mut Command) -> Value {
    let (code, out, err) = run(c);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
}

macro_rules! db {
    ($home:expr) => {
        openstance_core::db::open(
            &$home.join("graph.db"),
            &openstance_core::db::DbOptions {
                busy_timeout_ms: 5000,
            },
        )
        .unwrap()
    };
}

/// `init`, the test kit, and ties Priya-Marcus (email) and Marcus-Ravi (meeting).
fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = run(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
    assert_eq!(code, 0, "{err}");
    let conn = db!(dir.path());
    testkit::seed(&conn);
    openstance_core::search::fts::reindex_all(&conn).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
           (2, 3, 'email', 0.4, {}, 1),
           (3, 7, 'meeting', 0.5, {}, 1);",
        testkit::ago(60),
        testkit::ago(20)
    ))
    .unwrap();
    dir
}

// ---------------------------------------------------------------- treg-shaped server

struct Server {
    base: String,
    log: Arc<Mutex<Vec<(String, String, String)>>>,
}

fn respond(method: &str, path: &str) -> (u16, String, Option<u64>) {
    let catalog =
        |usd: f64| json!({ "routing": { "plan": [ { "endpoint_id": "x", "usd": usd } ] } });
    match (method, path) {
        ("GET", p) if p.starts_with("/catalog/endpoints/") => {
            (200, catalog(0.01).to_string(), None)
        }
        ("POST", "/call/treg.companies.enrich") => (
            200,
            json!({ "output": { "name": "Contoso", "domain": "contoso.example" },
                    "_treg": { "outcome": "hit", "provider": "alpha" } })
            .to_string(),
            Some(5_000),
        ),
        ("POST", "/call/treg.people.enrich") => (
            200,
            json!({
                "output": {
                    "full_name": "Dana Whitfield", "title": "CTO", "company": "Contoso",
                    "company_domain": "contoso.example", "linkedin_url": null, "location": null
                },
                "_treg": { "outcome": "hit", "provider": "alpha" }
            })
            .to_string(),
            Some(10_000),
        ),
        ("POST", "/call/treg.people.email.find") => (
            200,
            json!({ "output": { "email": "dana@contoso.example", "confidence": 0.9 },
                    "_treg": { "outcome": "hit", "provider": "beta" } })
            .to_string(),
            Some(8_000),
        ),
        ("POST", "/call/treg.people.email.verify") => (
            200,
            json!({ "output": { "valid": true, "status": "deliverable" },
                    "_treg": { "outcome": "hit", "provider": "gamma" } })
            .to_string(),
            Some(2_000),
        ),
        _ => (404, json!({ "detail": "no such tool" }).to_string(), None),
    }
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let log = log2.clone();
                std::thread::spawn(move || serve(stream, &log));
            }
        });
        Server { base, log }
    }

    fn requests(&self) -> Vec<(String, String, String)> {
        self.log.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<(String, String, String)>>) {
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
    let (status, body, cost) = respond(&method, &path);
    let mut out = stream;
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(c) = cost {
        head.push_str(&format!("X-Treg-Cost-Micro: {c}\r\n"));
    }
    head.push_str("\r\n");
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

// ---------------------------------------------------------------- path

#[test]
fn path_to_someone_in_the_graph_needs_no_egress() {
    let home = seeded();
    let v = ok_json(ost(home.path()).args(["--now", NOW, "--json", "path", "Marcus Lee"]));
    assert_eq!(v["status"], "done");
    assert_eq!(v["result"]["target"]["in_graph"], true);
    assert_eq!(
        v["result"]["paths"][0]["via"]["display_name"],
        "Priya Raman"
    );
    assert_eq!(v["receipt"]["requests_sent"], 0);

    let (code, out, _) = run(ost(home.path()).args(["--now", NOW, "path", "Marcus Lee"]));
    assert_eq!(code, 0);
    assert!(out.contains("Plan for path"), "{out}");
    assert!(out.contains("Priya Raman"), "{out}");
    assert!(out.contains("No egress"), "{out}");
}

#[test]
fn path_to_a_stranger_with_the_default_config_stays_local() {
    let home = seeded();
    let (code, out, err) = run(ost(home.path()).args([
        "--now",
        NOW,
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--find-email",
        "--allow-egress",
    ]));
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("enable an enricher"), "{err}");
    assert!(out.contains("skipped: no enricher"), "{out}");
    assert!(out.contains("Marcus Lee"), "{out}");
    let log = ok_json(ost(home.path()).args(["--json", "enrich", "log"]));
    assert_eq!(log["rows"], json!([]));
}

#[test]
fn path_quotes_first_then_spends_only_when_allowed() {
    let home = seeded();
    let server = Server::start();
    use_treg(home.path(), &server.base);
    let args = [
        "--now",
        NOW,
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--find-email",
    ];

    // 1. No approval and no TTY: the plan, the hint, exit 3, nothing sent.
    let (code, out, err) = run(ost(home.path()).args(args));
    assert_eq!(code, 3, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("enrich profile"), "{out}");
    assert!(out.contains("$0.0400"), "{out}");
    assert!(out.contains("Sends: company, domain, email, name"), "{out}");
    assert!(
        err.contains("re-run with --allow-egress to spend $0.0400"),
        "{err}"
    );
    assert!(server.requests().iter().all(|r| r.0 == "GET"));

    // 2. --json without approval: the Outcome with status quote, exit 3.
    let (code, out, _) = run(ost(home.path()).arg("--json").args(args));
    assert_eq!(code, 3);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "quote");
    // A profile by company name is two treg calls (company, then person).
    assert_eq!(v["plan"]["quote"]["total_micro_usd"], 40_000);

    // 3. Approved: exactly profile, find, verify; the actual costs are kept.
    let v = ok_json(
        ost(home.path())
            .arg("--json")
            .args(args)
            .arg("--allow-egress"),
    );
    assert_eq!(v["status"], "done", "{v}");
    assert_eq!(v["receipt"]["requests_sent"], 3);
    assert_eq!(v["receipt"]["spent_micro_usd"], 25_000);
    assert_eq!(v["result"]["paths"][0]["via"]["display_name"], "Marcus Lee");
    assert_eq!(v["result"]["email"]["address"], "dana@contoso.example");
    assert_eq!(v["result"]["email"]["status"], "deliverable");
    let posts: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|r| r.0 == "POST")
        .map(|r| r.1)
        .collect();
    assert_eq!(
        posts,
        vec![
            "/call/treg.companies.enrich",
            "/call/treg.people.enrich",
            "/call/treg.people.email.find",
            "/call/treg.people.email.verify"
        ]
    );

    // 4. The log shows what left, and the token never appears.
    let run_id = v["receipt"]["run_id"].as_str().unwrap().to_string();
    let log = ok_json(ost(home.path()).args(["--json", "enrich", "log", "--run", &run_id]));
    let statuses: Vec<&str> = log["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, vec!["ok", "ok", "ok"]);
    assert_eq!(log["totals"]["spent_micro_usd"], 25_000);
    assert_eq!(log["rows"][1]["request"]["domain"], "contoso.example");
    let (code, table, _) = run(ost(home.path()).args(["enrich", "log", "--since", "30d"]));
    assert_eq!(code, 0);
    assert!(table.contains("(Dana Whitfield)"), "{table}");
    assert!(table.contains("3 requests sent, $0.0250 spent"), "{table}");
    assert!(!table.contains(TOKEN) && !log.to_string().contains(TOKEN));
}

#[test]
fn approved_run_without_a_token_sends_nothing_and_counts_nothing() {
    let home = seeded();
    let server = Server::start();
    use_treg(home.path(), &server.base);
    let v = ok_json(ost(home.path()).env_remove("OST_TEST_TREG_TOKEN").args([
        "--json",
        "--now",
        NOW,
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--find-email",
        "--allow-egress",
    ]));
    assert_eq!(v["receipt"]["requests_sent"], 0, "{v}");
    assert_eq!(v["receipt"]["spent_micro_usd"], 0, "{v}");
    assert!(server.requests().iter().all(|r| r.0 == "GET"));
    let run_id = v["receipt"]["run_id"].as_str().unwrap().to_string();
    let log = ok_json(ost(home.path()).args(["--json", "enrich", "log", "--run", &run_id]));
    let rows = log["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "{log}");
    // The profile request was refused before it left; the email steps had
    // no input and stay quoted. Nothing reads as sent.
    assert_eq!(rows[0]["status"], "blocked", "{log}");
    assert!(
        rows.iter()
            .all(|r| r["status"] == "blocked" || r["status"] == "quoted"),
        "{log}"
    );
    assert!(
        rows[0]["error"]
            .as_str()
            .unwrap()
            .contains("OST_TEST_TREG_TOKEN"),
        "{log}"
    );
    assert_eq!(log["totals"]["requests_sent"], 0);
}

#[test]
fn plan_flag_prints_the_quote_and_exits_zero() {
    let home = seeded();
    let server = Server::start();
    use_treg(home.path(), &server.base);
    let (code, out, _) = run(ost(home.path()).args([
        "--now",
        NOW,
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--plan",
    ]));
    assert_eq!(code, 0);
    assert!(out.contains("Total $0.0200"), "{out}");
    assert!(server.requests().iter().all(|r| r.0 == "GET"));
}

// ---------------------------------------------------------------- brief

#[test]
fn brief_prints_pipe_friendly_markdown() {
    let home = seeded();
    let (code, out, err) = run(ost(home.path()).args(["--now", NOW, "brief", "Priya Raman"]));
    assert_eq!(code, 0, "{err}");
    let golden = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../openstance-core/tests/golden/brief_priya.md"),
    )
    .unwrap();
    assert_eq!(out, golden);
}

#[test]
fn brief_json_matches_the_shared_playbook_function() {
    let home = seeded();
    let v = ok_json(ost(home.path()).args(["--now", NOW, "--json", "brief", "Priya Raman"]));
    let g = Graph::open(
        Config::default(),
        &home.path().join("graph.db"),
        Providers::null(),
        Box::new(FixedClock(testkit::NOW)),
    )
    .unwrap();
    let direct = g
        .run_playbook(
            "brief",
            &json!({"person": "Priya Raman", "web": false}),
            &RunOptions::default(),
        )
        .unwrap();
    assert_eq!(v["result"], serde_json::to_value(&direct.result).unwrap());
    assert_eq!(v["status"], "done");
}

#[test]
fn brief_web_search_needs_approval() {
    let home = seeded();
    let server = Server::start();
    use_treg(home.path(), &server.base);
    let (code, out, err) =
        run(ost(home.path()).args(["--now", NOW, "brief", "Priya Raman", "--web"]));
    assert_eq!(code, 3, "{err}");
    assert!(out.starts_with("**Priya Raman**"), "{out}");
    assert!(err.contains("web search for recent posts"), "{err}");
    assert!(server.requests().iter().all(|r| r.0 == "GET"));
}

#[test]
fn enrich_log_is_empty_on_a_fresh_graph() {
    let home = seeded();
    let (code, out, _) = run(ost(home.path()).args(["enrich", "log"]));
    assert_eq!(code, 0);
    assert!(out.contains("nothing has left this machine"), "{out}");
}
