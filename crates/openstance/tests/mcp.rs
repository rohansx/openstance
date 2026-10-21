//! `openstance mcp` end to end (P2-CLI-04, P2-MCP-02 acceptance): the real
//! binary speaks newline-delimited JSON-RPC on stdio against a seeded home.
//! stdout must carry nothing but protocol messages, and `search_people`
//! must return the same document as `openstance search --json`.

#![cfg(feature = "server")]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use openstance_core::search::testkit;
use serde_json::{json, Value};

const NOW: &str = "2026-10-20T12:00:00Z";

fn env(c: &mut Command, home: &Path) {
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env("NO_COLOR", "1");
}

fn cli(home: &Path, args: &[&str]) -> Value {
    let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
    env(&mut c, home);
    c.env_remove("OPENSTANCE_LOG");
    let out = c.args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A home with `init` run and the test-kit graph seeded and indexed.
fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cli(dir.path(), &["init", "--me", "owner@example.com", "--json"]);
    let conn = openstance_core::db::open(
        &dir.path().join("graph.db"),
        &openstance_core::db::DbOptions {
            busy_timeout_ms: 5000,
        },
    )
    .unwrap();
    testkit::seed(&conn);
    openstance_core::search::fts::reindex_all(&conn).unwrap();
    dir
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    /// Every stdout line seen so far.
    lines: Vec<String>,
    next_id: i64,
}

impl Server {
    fn start(home: &Path, extra: &[&str]) -> Server {
        let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
        env(&mut c, home);
        // Turn logging all the way up: it must still never reach stdout.
        c.env("OPENSTANCE_LOG", "trace")
            .args(["mcp", "--now", NOW])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Server {
            child,
            stdin,
            stdout,
            lines: Vec::new(),
            next_id: 1,
        };
        let init = s.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "openstance-test", "version": "0" }
            }),
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

    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line).unwrap() == 0 {
            return None;
        }
        self.lines.push(line.clone());
        Some(line)
    }

    /// Send a request and return its response (skipping notifications).
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self.read_line().expect("server closed stdout");
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue; // reported by assert_protocol_only
            };
            if v["id"] == id {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        r["result"].clone()
    }

    /// Close stdin, wait for exit; return (exit success, stderr).
    fn finish(mut self) -> (Vec<String>, bool, String) {
        drop(self.stdin.take());
        while self.read_line().is_some() {}
        let out = self.child.wait_with_output().unwrap();
        (
            self.lines,
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

fn redact(mut v: Value) -> Value {
    if let Some(m) = v.as_object_mut() {
        m.remove("took_ms");
        m.remove("timings_ms");
    }
    v
}

#[test]
fn stdout_carries_only_json_rpc() {
    let home = seeded();
    let mut s = Server::start(home.path(), &[]);
    let tools = s.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 3);
    let r = s.call("search_people", json!({"text": "devtools dinner"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = s.call("get_person", json!({"person": "Priya Raman"}));
    assert_eq!(r["structuredContent"]["person"]["id"], 2, "{r}");
    let r = s.call("how_connected", json!({"target": "Sam Ito"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = s.call("get_person", json!({"person": "Nobody Here"}));
    assert_eq!(r["isError"], true, "{r}");
    let (lines, ok, stderr) = s.finish();
    assert!(ok, "clean exit on stdin EOF: {stderr}");
    assert!(lines.len() >= 6, "{lines:?}");
    for line in &lines {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-JSON on stdout ({e}): {line:?}"));
        assert_eq!(v["jsonrpc"], "2.0", "{line}");
        assert!(
            v.get("result").is_some() || v.get("error").is_some() || v.get("method").is_some(),
            "{line}"
        );
    }
    assert!(!stderr.trim().is_empty(), "logs go to stderr");
}

#[test]
fn search_people_is_byte_identical_to_search_json() {
    let home = seeded();
    let h = home.path();
    let cases: [(&[&str], Value); 3] = [
        (
            &["search", "devtools dinner", "--limit", "5"],
            json!({"text": "devtools dinner", "limit": 5}),
        ),
        (
            &["search", "--company", "Contoso", "--met", "--limit", "20"],
            json!({"company": ["Contoso"], "met_in_person": true}),
        ),
        (
            &[
                "search",
                "founders",
                "--since",
                "2025",
                "--no-parse",
                "--include-bulk",
                "--limit",
                "10",
            ],
            json!({"text": "founders", "since": "2025", "parse": false, "include_bulk": true, "limit": 10}),
        ),
    ];
    let mut s = Server::start(h, &[]);
    for (flags, args) in cases {
        let mut argv: Vec<&str> = flags.to_vec();
        argv.extend(["--json", "--now", NOW]);
        let expected = redact(cli(h, &argv));
        let got = redact(s.call("search_people", args.clone())["structuredContent"].clone());
        assert_eq!(
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&expected).unwrap(),
            "{args}"
        );
    }
    s.finish();
}

#[test]
fn read_only_flag_is_accepted() {
    let home = seeded();
    let mut s = Server::start(home.path(), &["--read-only"]);
    let r = s.call("search_people", json!({"text": "engineer"}));
    assert_eq!(r["isError"], false, "{r}");
    let (_, ok, stderr) = s.finish();
    assert!(ok, "{stderr}");
}

#[test]
fn mcp_help_mentions_stdio() {
    let out = Command::new(env!("CARGO_BIN_EXE_openstance"))
        .args(["mcp", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("stdio"), "{help}");
}

#[test]
fn get_person_matches_who_json() {
    let home = seeded();
    let h = home.path();
    let mut s = Server::start(h, &[]);
    for person in ["2", "Priya Raman", "Sam Ito"] {
        let expected = cli(h, &["who", person, "--json", "--now", NOW]);
        let got = s.call("get_person", json!({"person": person}))["structuredContent"].clone();
        assert_eq!(
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&expected).unwrap(),
            "{person}"
        );
    }
    s.finish();
}

#[test]
fn how_connected_is_byte_identical_to_how_json() {
    let home = seeded();
    let h = home.path();
    let mut s = Server::start(h, &[]);
    let cases: [(&[&str], Value); 3] = [
        (&["how", "Sam Ito"], json!({"target": "Sam Ito"})),
        (
            &["how-connected", "Priya Raman", "--limit", "1"],
            json!({"target": "Priya Raman", "limit": 1}),
        ),
        (&["how", "Nobody Here"], json!({"target": "Nobody Here"})),
    ];
    for (flags, args) in cases {
        let mut argv: Vec<&str> = flags.to_vec();
        argv.extend(["--json", "--now", NOW]);
        let expected = redact(cli(h, &argv));
        let got = redact(s.call("how_connected", args.clone())["structuredContent"].clone());
        assert_eq!(
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&expected).unwrap(),
            "{args}"
        );
    }
    s.finish();
}

/// With vectors built (the deterministic `hash:<dims>` embedder, no model
/// download), `search_people` fuses FTS and vectors exactly like
/// `openstance search --json` (P2-CORE-03 through MCP).
#[test]
fn search_people_matches_search_json_in_hybrid_mode() {
    let home = seeded();
    let h = home.path();
    let cfg_path = h.join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap().replace(
        "# embedder = \"fastembed:bge-small-en-v1.5\"",
        "embedder = \"hash:64\"",
    );
    assert!(cfg.contains("embedder = \"hash:64\""));
    std::fs::write(&cfg_path, cfg).unwrap();
    cli(h, &["rebuild", "--embeddings", "--json", "--now", NOW]);
    let mut s = Server::start(h, &[]);
    for (flags, args) in [
        (
            vec!["search", "devtools dinner", "--limit", "5"],
            json!({"text": "devtools dinner", "limit": 5}),
        ),
        (
            vec!["search", "board deck", "--limit", "10"],
            json!({"text": "board deck", "limit": 10}),
        ),
    ] {
        let mut argv = flags;
        argv.extend(["--json", "--now", NOW]);
        let expected = redact(cli(h, &argv));
        assert_eq!(expected["models"]["embedder"], "hash:64", "{expected}");
        let fused = expected["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| !r["relevance"]["vector"].is_null());
        assert!(fused, "vectors took part: {expected}");
        let got = redact(s.call("search_people", args.clone())["structuredContent"].clone());
        assert_eq!(
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&expected).unwrap(),
            "{args}"
        );
    }
    s.finish();
}
