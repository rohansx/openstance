//! `openstance serve` end to end (P4-API-01/02/03/06, P4-CLI-01): the real
//! binary on a free loopback port against a seeded home. The HTTP JSON must
//! equal the CLI's `--json` output for the same input and `--now` (contract
//! test), the token file must be private, and non-loopback binds need the
//! explicit flag.

#![cfg(feature = "server")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use openstance_core::search::testkit;
use serde_json::Value;

const NOW: &str = "2026-10-20T12:00:00Z";

fn env(c: &mut Command, home: &Path) {
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("NO_COLOR", "1");
}

fn cli_raw(home: &Path, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
    env(&mut c, home);
    c.args(args).output().unwrap()
}

fn cli(home: &Path, args: &[&str]) -> Value {
    let out = cli_raw(home, args);
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
    port: u16,
    token: String,
    ready: Value,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start(home: &Path, extra: &[&str]) -> Server {
        let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
        env(&mut c, home);
        c.args(["serve", "--port", "0", "--json", "--now", NOW])
            .args(extra)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let ready: Value = serde_json::from_str(&line).unwrap_or_else(|e| {
            let mut err = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            panic!("no ready line ({e}): {line:?} {err}")
        });
        let mcp = ready["mcp"].as_str().unwrap();
        let port: u16 = mcp
            .trim_end_matches("/mcp")
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let token = std::fs::read_to_string(home.join("api.token"))
            .unwrap()
            .trim()
            .to_string();
        if let Some(ui) = ready["ui"].as_str() {
            assert_eq!(ui.split("#token=").nth(1), Some(token.as_str()), "{ready}");
        }
        Server {
            child,
            port,
            token,
            ready,
        }
    }

    /// One HTTP/1.1 request; returns (status, body).
    fn request(&self, method: &str, path: &str, body: Option<&str>, auth: bool) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
            self.port
        );
        if auth {
            req.push_str(&format!("Authorization: Bearer {}\r\n", self.token));
        }
        if let Some(b) = body {
            req.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                b.len()
            ));
        }
        req.push_str("\r\n");
        if let Some(b) = body {
            req.push_str(b);
        }
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("http head");
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut body = raw[split + 4..].to_vec();
        if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            body = dechunk(&body);
        }
        (status, body)
    }

    fn get(&self, path: &str) -> Value {
        let (status, body) = self.request("GET", path, None, true);
        assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
        serde_json::from_slice(&body).unwrap()
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        out.extend_from_slice(&b[nl + 2..nl + 2 + size]);
        b = &b[nl + 2 + size + 2..];
    }
    out
}

fn no_timing(mut v: Value) -> Value {
    v["took_ms"] = Value::Null;
    v
}

#[test]
fn http_json_equals_cli_json() {
    let home = seeded();
    let h = home.path();
    let s = Server::start(h, &[]);
    assert!(s.token.len() >= 43, "{}", s.ready);

    // search: explicit limit, default limit, structured filters.
    let cli_search = cli(
        h,
        &[
            "search",
            "northwind",
            "--limit",
            "5",
            "--json",
            "--now",
            NOW,
        ],
    );
    let http_search = s.get("/people?q=northwind&limit=5");
    assert_eq!(no_timing(http_search), no_timing(cli_search));
    let cli_search = cli(h, &["search", "contoso", "--json", "--now", NOW]);
    assert_eq!(no_timing(s.get("/people?q=contoso")), no_timing(cli_search));
    let cli_search = cli(
        h,
        &[
            "search",
            "--company",
            "Northwind",
            "--met",
            "--since",
            "2025",
            "--json",
            "--now",
            NOW,
        ],
    );
    assert_eq!(
        no_timing(s.get("/people?company=Northwind&met=true&since=2025")),
        no_timing(cli_search)
    );

    // who / get_person
    let cli_who = cli(h, &["who", "2", "--json", "--now", NOW]);
    assert_eq!(s.get("/people/2"), cli_who);

    // how / connections
    let cli_how = cli(h, &["how", "3", "--json", "--now", NOW]);
    assert_eq!(s.get("/connections?target=3"), cli_how);

    // review --list
    let cli_review = cli(h, &["review", "--list", "--json", "--now", NOW]);
    assert_eq!(s.get("/review"), cli_review);

    // enrich log
    let cli_log = cli(h, &["enrich", "log", "--json", "--now", NOW]);
    assert_eq!(s.get("/enrich-log"), cli_log);

    // config
    let cli_config = cli(h, &["config", "--json"]);
    assert_eq!(s.get("/config"), cli_config);
    let served = s.get("/config");
    let db = served["db_path"]
        .as_str()
        .expect("GET /config carries db_path");
    assert!(db.ends_with("graph.db") && !db.starts_with('~'), "{db}");
    assert!(served["home"].is_string());

    // view list
    let cli_views = cli(h, &["view", "list", "--json", "--now", NOW]);
    assert_eq!(s.get("/views"), cli_views);

    // path playbook: default egress returns the quote (or nothing to do)
    // and sends nothing.
    let (status, body) = s.request(
        "POST",
        "/playbooks/path",
        Some(r#"{"input":{"target":"Priya Raman"}}"#),
        true,
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let out: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(out["receipt"]["requests_sent"], 0);

    // Phase 4 playbooks: the same `result` from CLI --json and HTTP (the
    // run id and quote time differ per run, the result does not).
    let targets = h.join("targets.txt");
    std::fs::write(&targets, "Contoso\nnorthwind.example\n").unwrap();
    for (name, args, body) in [
        (
            "reconnect",
            vec!["reconnect"],
            r#"{"input":{}}"#.to_string(),
        ),
        (
            "accounts",
            vec!["accounts", targets.to_str().unwrap()],
            r#"{"input":{"companies":["Contoso","northwind.example"],"one_hop":true}}"#.to_string(),
        ),
    ] {
        let mut a = args.clone();
        a.extend(["--json", "--now", NOW]);
        let from_cli = cli(h, &a);
        let (status, bytes) = s.request("POST", &format!("/playbooks/{name}"), Some(&body), true);
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        let from_http: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(from_http["result"], from_cli["result"], "{name}");
        assert_eq!(from_http["status"], from_cli["status"], "{name}");
    }
}

#[test]
fn health_token_and_ui() {
    let home = seeded();
    let h = home.path();
    let s = Server::start(h, &[]);
    let (status, body) = s.request("GET", "/health", None, false);
    assert_eq!(status, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["schema_version"], openstance_core::db::LATEST_VERSION);
    let (status, _) = s.request("GET", "/stats", None, false);
    assert_eq!(status, 401);
    let (status, body) = s.request("GET", "/", None, false);
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body)
        .to_ascii_lowercase()
        .contains("<html"));
    assert_eq!(s.ready["mcp"], format!("http://127.0.0.1:{}/mcp", s.port));

    // The token file is private and is the one handed to the UI.
    let file = std::fs::read_to_string(h.join("api.token")).unwrap();
    assert_eq!(file.trim(), s.token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(h.join("api.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn init_creates_the_token_and_print_token_prints_it() {
    let home = seeded();
    let h = home.path();
    assert!(h.join("api.token").exists(), "init writes api.token");
    let out = cli_raw(h, &["serve", "--print-token"]);
    assert!(out.status.success());
    let printed = String::from_utf8(out.stdout).unwrap();
    let file = std::fs::read_to_string(h.join("api.token")).unwrap();
    assert_eq!(printed.trim(), file.trim());

    let out = cli_raw(h, &["serve", "--rotate-token", "--print-token"]);
    assert!(out.status.success());
    let rotated = String::from_utf8(out.stdout).unwrap();
    assert_ne!(rotated.trim(), printed.trim());
    assert_eq!(
        std::fs::read_to_string(h.join("api.token")).unwrap().trim(),
        rotated.trim()
    );
}

#[test]
fn non_loopback_bind_needs_the_flag() {
    let home = seeded();
    let h = home.path();
    let out = cli_raw(h, &["serve", "--host", "0.0.0.0", "--port", "0"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--i-understand-this-exposes-my-graph"),
        "{err}"
    );

    // serve.host in config alone is not enough.
    std::fs::write(h.join("config.toml"), "[serve]\nhost = \"0.0.0.0\"\n").unwrap();
    let out = cli_raw(h, &["serve", "--port", "0"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn mcp_only_and_no_ui() {
    let home = seeded();
    let h = home.path();
    let s = Server::start(h, &["--mcp-only"]);
    let (status, _) = s.request("GET", "/stats", None, true);
    assert_eq!(status, 404);
    assert!(s.ready["ui"].is_null());
    drop(s);
    let s = Server::start(h, &["--no-ui"]);
    let (status, _) = s.request("GET", "/", None, false);
    assert_eq!(status, 404);
    let (status, _) = s.request("GET", "/stats", None, true);
    assert_eq!(status, 200);
}
