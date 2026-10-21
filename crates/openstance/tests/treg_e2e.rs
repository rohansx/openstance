//! Live check against a local treg registry (P3-ENR-04, phase-3 exit
//! criterion 4 dry run). Ignored by default and never run in CI:
//!
//! ```sh
//! docker compose -f dev/treg/compose.yaml up -d
//! OPENSTANCE_TREG_E2E=1 cargo test -p openstance --test treg_e2e -- --ignored --nocapture
//! ```
//!
//! - `OPENSTANCE_TREG_E2E_URL` overrides the registry (default
//!   `http://127.0.0.1:18790`).
//! - Quoting needs no token: catalog reads are open, and the tests assert
//!   that nothing is sent and nothing is spent.
//! - The approved (`--allow-egress`) leg runs with the token from
//!   `OPENSTANCE_TREG_E2E_TOKEN` when it is set. A self-hosted registry
//!   without provider keys refuses the call (402 / 422); the test asserts
//!   openstance surfaces that, logs an `error` row and writes nothing to the
//!   graph. Every call carries `X-Treg-Route-Max-Cost` equal to the quote,
//!   so a registry that does have keys can spend at most the quote. Without
//!   the token the same leg checks the missing-token path, which fails
//!   before any request leaves the machine.

use std::path::Path;
use std::process::Command;

use openstance_core::search::testkit;
use serde_json::{json, Value};

const NOW: &str = "2026-10-20T12:00:00Z";

fn enabled() -> bool {
    if std::env::var("OPENSTANCE_TREG_E2E").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: set OPENSTANCE_TREG_E2E=1 (and start dev/treg) to run");
    false
}

fn endpoint() -> String {
    std::env::var("OPENSTANCE_TREG_E2E_URL").unwrap_or_else(|_| "http://127.0.0.1:18790".into())
}

fn token() -> Option<String> {
    std::env::var("OPENSTANCE_TREG_E2E_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
}

fn ost(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env_remove("OPENSTANCE_TREG_TOKEN")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("NO_COLOR", "1")
        .args(["--now", NOW]);
    c
}

fn run(c: &mut Command) -> (i32, String, String) {
    let out = c.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
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

/// A seeded home whose config selects the treg backend at [`endpoint`].
fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (code, _, err) = run(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
    assert_eq!(code, 0, "{err}");
    let conn = db(dir.path());
    testkit::seed(&conn);
    openstance_core::search::fts::reindex_all(&conn).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!(
            "[me]\nidentities = [\"email:owner@example.com\"]\n\n[enrich]\nbackend = \"treg\"\n\n\
             [enrich.treg]\nendpoint = \"{}\"\ntoken_env = \"OPENSTANCE_TREG_E2E_TOKEN\"\n",
            endpoint()
        ),
    )
    .unwrap();
    dir
}

fn log_rows(home: &Path) -> Vec<(String, String, Option<i64>, Option<String>)> {
    db(home)
        .prepare("SELECT op, status, cost_micro_usd, error FROM enrich_log ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(home: &Path, sql: &str) -> i64 {
    db(home).query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `quote_enrich` over MCP and `path --plan`: real catalog prices, no
/// spend, only `quoted` rows.
#[test]
#[ignore = "live: needs a local treg registry; OPENSTANCE_TREG_E2E=1"]
fn quotes_come_from_the_live_catalog_and_spend_nothing() {
    if !enabled() {
        return;
    }
    let h = home();
    let home = h.path();

    // MCP quote_enrich, one request per kind, through the real binary.
    let mut child = ost(home)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"e2e","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"quote_enrich",
            "arguments":{"requests":[
                {"kind":"profile","name":"Dana Whitfield","linkedin":"dana-whitfield"},
                {"kind":"find_email","name":"Dana Whitfield","domain":"contoso.example"},
                {"kind":"verify_email","email":"dana@contoso.example"},
                {"kind":"company","domain":"contoso.example"},
                {"kind":"web_search","query":"Dana Whitfield Contoso"}]}}}),
    ];
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        for m in &msgs {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let quote = {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut found = Value::Null;
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            let v: Value = serde_json::from_str(&line).unwrap();
            line.clear();
            if v["id"] == 2 {
                found = v["result"].clone();
                break;
            }
        }
        drop(child.stdin.take());
        let _ = child.wait();
        found
    };
    assert_eq!(quote["isError"], false, "{quote}");
    let q = &quote["structuredContent"];
    println!(
        "quote_enrich via {}: {}",
        endpoint(),
        serde_json::to_string_pretty(q).unwrap()
    );
    assert_eq!(q["backend"], "treg");
    let lines = q["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 5, "every kind priced: {q}");
    for l in lines {
        assert!(l["micro_usd"].as_u64().unwrap() > 0, "{l}");
        assert_eq!(
            l["basis"], "estimate",
            "routed price is the upper bound: {l}"
        );
    }

    // `path --plan`: the plan and quote, exit 0, nothing sent.
    let (code, out, err) = run(ost(home).args([
        "--json",
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--find-email",
        "--plan",
    ]));
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    let plan: Value = serde_json::from_str(&out).unwrap();
    println!(
        "path --plan quote: {} micro-USD for {} request(s)",
        plan["plan"]["quote"]["total_micro_usd"],
        plan["plan"]["quote"]["lines"]
            .as_array()
            .map_or(0, Vec::len)
    );
    assert_eq!(plan["status"], "quote");
    assert_eq!(plan["plan"]["quote"]["backend"], "treg");
    assert!(plan["plan"]["quote"]["total_micro_usd"].as_u64().unwrap() > 0);
    assert_eq!(plan["receipt"]["requests_sent"], 0);
    assert_eq!(plan["receipt"]["spent_micro_usd"], 0);

    let rows = log_rows(home);
    assert!(!rows.is_empty());
    assert!(
        rows.iter().all(|r| r.1 == "quoted" && r.2.is_none()),
        "only quoted rows, no cost: {rows:?}"
    );
}

/// An approved `path` against the live registry. Without provider keys (or
/// without a token) it cannot succeed; openstance must report the failure,
/// log it, and leave the graph untouched.
#[test]
#[ignore = "live: needs a local treg registry; OPENSTANCE_TREG_E2E=1"]
fn an_approved_call_that_the_registry_refuses_is_handled_cleanly() {
    if !enabled() {
        return;
    }
    let h = home();
    let home = h.path();
    let people = count(home, "SELECT count(*) FROM person");
    let mut cmd = ost(home);
    cmd.args([
        "--json",
        "path",
        "Dana Whitfield",
        "--company",
        "Contoso",
        "--allow-egress",
        // Live upper-bound prices put a profile-by-company-name at two
        // routed calls (~$0.76), over the default $0.50 budget; the CLI may
        // raise the budget for one run. The call is capped at the quote.
        "--max-usd",
        "2",
    ]);
    match token() {
        Some(t) => {
            cmd.env("OPENSTANCE_TREG_E2E_TOKEN", t);
        }
        None => {
            cmd.env_remove("OPENSTANCE_TREG_E2E_TOKEN");
        }
    }
    let (code, out, err) = run(&mut cmd);
    println!("exit {code}\nstderr: {err}\nstdout: {out}");
    let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    let rows = log_rows(home);
    println!("enrich_log: {rows:?}");
    let quoted = v["plan"]["quote"]["total_micro_usd"].as_i64().unwrap();
    let spent = v["receipt"]["spent_micro_usd"].as_i64().unwrap();
    assert!(spent <= quoted, "never more than the quote");
    if v["status"] == "done" {
        // A registry with working provider keys answered: within the quote.
        assert!(rows.iter().all(|r| r.1 == "ok" || r.1 == "not_found"));
        return;
    }
    assert_eq!(v["status"], "partial", "{v}");
    assert!(
        !v["notes"].as_array().unwrap().is_empty(),
        "the failure is reported"
    );
    assert!(
        rows.iter().any(|r| r.1 == "error" && r.3.is_some()),
        "an error row with its message: {rows:?}"
    );
    assert!(
        rows.iter().all(|r| r.1 != "sent" && r.1 != "ok"),
        "{rows:?}"
    );
    assert_eq!(
        count(home, "SELECT count(*) FROM person"),
        people,
        "no partial writes"
    );
    assert_eq!(
        count(
            home,
            "SELECT count(*) FROM source WHERE kind LIKE 'enrich:%'"
        ),
        0,
        "nothing committed"
    );
    if let Some(t) = token() {
        assert!(
            !out.contains(&t) && !err.contains(&t),
            "token never printed"
        );
    }
}
