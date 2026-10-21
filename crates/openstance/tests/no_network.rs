//! Phase 3 exit criterion 6: with the default config (`enrich.backend =
//! "none"`), no command, including `path` and `brief` with egress allowed,
//! opens a network socket. Each command runs with the network blocked for
//! the process: `sandbox-exec` with `(deny network*)` on macOS, `unshare -rn`
//! (an empty network namespace) on Linux. Where neither is available the
//! test says so and returns.
//!
//! A control case first proves the block is real: the same sandbox makes a
//! treg-configured `path` fail to reach a listening local server.
//!
//! The embedder is pinned to `none` (as in every CLI test): the default
//! local embedder downloads its model once on first use, which is a
//! separate, documented network use (Phase 2, `demo.sh --preflight`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

const NOW: &str = "2026-10-15";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// How to run a child with no network, if this machine can.
fn sandbox() -> Option<Vec<String>> {
    let works = |argv: &[&str]| {
        Command::new(argv[0])
            .args(&argv[1..])
            .arg("/usr/bin/true")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let mac = [
        "sandbox-exec",
        "-p",
        "(version 1)(allow default)(deny network*)",
    ];
    if cfg!(target_os = "macos") && works(&mac) {
        return Some(mac.iter().map(|s| (*s).to_string()).collect());
    }
    let linux = ["unshare", "-rn"];
    if cfg!(target_os = "linux") && works(&linux) {
        return Some(linux.iter().map(|s| (*s).to_string()).collect());
    }
    None
}

fn ost(sb: &[String], home: &Path) -> Command {
    let mut c = Command::new(&sb[0]);
    c.args(&sb[1..])
        .arg(env!("CARGO_BIN_EXE_openstance"))
        .env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env_remove("OPENSTANCE_TREG_TOKEN")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OST_TEST_TREG_TOKEN", "t")
        .env("NO_COLOR", "1")
        .args(["--now", NOW]);
    c
}

fn run(sb: &[String], home: &Path, args: &[&str]) -> Output {
    ost(sb, home).args(args).output().unwrap()
}

fn expect(sb: &[String], home: &Path, args: &[&str], code: i32) -> String {
    let o = run(sb, home, args);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(
        o.status.code(),
        Some(code),
        "{args:?}\nstdout: {}\nstderr: {err}",
        String::from_utf8_lossy(&o.stdout)
    );
    let lower = err.to_lowercase();
    for bad in [
        "operation not permitted",
        "connection refused",
        "network",
        "dns",
    ] {
        assert!(!lower.contains(bad), "{args:?} touched the network: {err}");
    }
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_sandbox_really_blocks_the_binarys_sockets() {
    let Some(sb) = sandbox() else {
        eprintln!("skipped: no sandbox-exec / unshare -rn on this machine");
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    expect(&sb, h, &["init", "--me", "you@example.com"], 0);
    std::fs::write(
        h.join("config.toml"),
        format!(
            "[enrich]\nbackend = \"treg\"\n[enrich.treg]\nendpoint = \"{base}\"\ntoken_env = \"OST_TEST_TREG_TOKEN\"\n"
        ),
    )
    .unwrap();
    let o = run(
        &sb,
        h,
        &[
            "path",
            "Noor Haddad",
            "--company",
            "Lumen Pay",
            "--allow-egress",
        ],
    );
    assert!(
        !o.status.success(),
        "with treg configured, pricing must fail when the network is blocked: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    listener.set_nonblocking(true).unwrap();
    assert!(
        listener.accept().is_err(),
        "no connection reached the listener"
    );
}

#[test]
fn default_config_opens_no_socket_in_any_command() {
    let Some(sb) = sandbox() else {
        eprintln!("skipped: no sandbox-exec / unshare -rn on this machine");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let demo = |f: &str| repo(&format!("fixtures/demo/{f}")).display().to_string();
    let (mbox, csv, vcf) = (
        demo("takeout.mbox"),
        demo("Connections.csv"),
        demo("contacts.vcf"),
    );

    expect(&sb, h, &["init", "--me", "you@example.com"], 0);
    expect(&sb, h, &["import", &mbox, &csv, &vcf], 0);
    expect(&sb, h, &["search", "fintech founders who replied"], 0);
    expect(&sb, h, &["who", "Maya Chen"], 0);
    expect(&sb, h, &["how", "Dana Whitfield"], 0);

    // The paid playbooks, with egress allowed: still nothing leaves.
    let out = expect(
        &sb,
        h,
        &[
            "--json",
            "path",
            "Noor Haddad",
            "--company",
            "Lumen Pay",
            "--find-email",
            "--allow-egress",
        ],
        0,
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["plan"]["quote"]["backend"], "none");
    assert_eq!(v["receipt"]["requests_sent"], 0);
    expect(&sb, h, &["path", "Noor Haddad", "--plan"], 0);
    let out = expect(
        &sb,
        h,
        &["--json", "brief", "Maya Chen", "--web", "--allow-egress"],
        0,
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["receipt"]["requests_sent"], 0);
    let out = expect(&sb, h, &["--json", "enrich", "log"], 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["rows"], json!([]), "backend none logs nothing");

    // Sharing and export.
    expect(&sb, h, &["keys", "show"], 0);
    expect(
        &sb,
        h,
        &[
            "view",
            "create",
            "investors",
            "--query",
            r#"{"title":["Venture Partner"]}"#,
            "--ttl",
            "30d",
        ],
        0,
    );
    let ost_file = h.join("investors.ost").display().to_string();
    expect(&sb, h, &["view", "export", "investors", "-o", &ost_file], 0);
    expect(&sb, h, &["view", "list"], 0);
    let vcf_out = h.join("out.vcf").display().to_string();
    expect(&sb, h, &["export", "--format", "vcf", "-o", &vcf_out], 0);

    // The MCP server and its paid tools.
    let mut child = ost(&sb, h)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"t","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"path_to",
            "arguments":{"target":"Noor Haddad","company":"Lumen Pay","find_email":true,
                         "allow_egress":true}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"brief",
            "arguments":{"person":"Maya Chen","web":true,"allow_egress":true}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"quote_enrich",
            "arguments":{"requests":[{"kind":"verify_email","email":"a@b.example"}]}}}),
    ];
    for m in &msgs {
        writeln!(stdin, "{m}").unwrap();
    }
    stdin.flush().unwrap();
    let mut seen = 0;
    let mut line = String::new();
    while seen < 4 && reader.read_line(&mut line).unwrap() > 0 {
        let v: Value = serde_json::from_str(&line).unwrap();
        line.clear();
        if let Some(id) = v["id"].as_i64() {
            seen += 1;
            if id > 1 {
                assert_eq!(v["result"]["isError"], false, "{v}");
                let sc = &v["result"]["structuredContent"];
                if id < 4 {
                    assert_eq!(sc["receipt"]["requests_sent"], 0, "{v}");
                } else {
                    assert_eq!(sc["total_micro_usd"], 0, "{v}");
                }
            }
        }
    }
    assert_eq!(seen, 4);
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
