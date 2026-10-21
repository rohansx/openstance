//! The Phase 3 demo script (docs/plan/phase-3.md, "Demo script") run with
//! the real binary on the fictional dataset in fixtures/demo/. Paid steps go
//! to an in-process HTTP server that answers like a treg router with canned
//! bodies written for this demo, so nothing touches the internet.
//!
//! `cargo test -p openstance --test demo_phase3 -- --nocapture` prints the
//! transcript.
//!
//! Dataset note: fixtures/demo already holds Dana Whitfield (the Phase 2
//! `how` demo), so the "not in my graph" target here is Noor Haddad, a
//! fictional newcomer at Lumen Pay. Dana is shown as the in-graph case.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

const NOW: &str = "2026-10-15";
const TOKEN: &str = "demo-token-not-a-secret";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn ost(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_openstance"));
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OST_DEMO_TREG_TOKEN", TOKEN)
        .env("NO_COLOR", "1")
        .args(["--now", NOW]);
    c
}

/// Run `ost <args>` and print it like a terminal session.
fn show(home: &Path, who: &str, args: &[&str]) -> (i32, String, String) {
    let out = ost(home).args(args).output().unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let shown: Vec<String> = args
        .iter()
        .map(|a| {
            if a.contains(' ') || a.contains('{') {
                format!("'{a}'")
            } else {
                (*a).to_string()
            }
        })
        .collect();
    println!("{who}$ ost {}", shown.join(" "));
    print!("{stdout}");
    for l in stderr.lines() {
        println!("  [stderr] {l}");
    }
    if code != 0 {
        println!("  [exit {code}]");
    }
    println!();
    (code, stdout, stderr)
}

// ------------------------------------------------------------ canned router

type Log = Arc<Mutex<Vec<(String, String)>>>;

fn price(id: &str) -> f64 {
    match id {
        "treg.people.enrich" => 0.010,
        "treg.companies.enrich" => 0.002,
        "treg.people.email.find" => 0.004,
        "treg.people.email.verify" => 0.002,
        _ => 0.005,
    }
}

fn respond(method: &str, path: &str, body: &[u8]) -> (u16, String, Option<u64>) {
    let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let name = req["full_name"]
        .as_str()
        .unwrap_or("Noor Haddad")
        .to_string();
    let first = name.split_whitespace().next().unwrap_or("x").to_lowercase();
    match (method, path) {
        ("GET", p) if p.starts_with("/catalog/endpoints/") => {
            let id = p.trim_start_matches("/catalog/endpoints/");
            let usd = price(id);
            (
                200,
                json!({ "endpoint": { "id": id, "cost": { "usd": usd } },
                        "routing": { "plan": [ { "endpoint_id": "demo", "usd": usd } ] } })
                .to_string(),
                None,
            )
        }
        ("POST", "/call/treg.companies.enrich") => (
            200,
            json!({ "output": { "name": "Lumen Pay", "domain": "lumenpay.example" },
                    "_treg": { "outcome": "hit", "provider": "demo" } })
            .to_string(),
            Some(2_000),
        ),
        ("POST", "/call/treg.people.enrich") => (
            200,
            json!({ "output": { "full_name": name, "title": "Head of Risk",
                                "company": "Lumen Pay", "company_domain": "lumenpay.example",
                                "linkedin_url": null, "location": "London" },
                    "_treg": { "outcome": "hit", "provider": "demo" } })
            .to_string(),
            Some(10_000),
        ),
        ("POST", "/call/treg.people.email.find") => (
            200,
            json!({ "output": { "email": format!("{first}@lumenpay.example"), "confidence": 0.92 },
                    "_treg": { "outcome": "hit", "provider": "demo" } })
            .to_string(),
            Some(4_000),
        ),
        ("POST", "/call/treg.people.email.verify") => (
            200,
            json!({ "output": { "valid": true, "status": "deliverable", "score": 0.97 },
                    "_treg": { "outcome": "hit", "provider": "demo" } })
            .to_string(),
            Some(2_000),
        ),
        _ => (404, json!({ "detail": "no such tool" }).to_string(), None),
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

fn handle(stream: TcpStream, log: &Mutex<Vec<(String, String)>>) {
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
    log.lock().unwrap().push((method.clone(), path.clone()));
    let (status, body, cost) = respond(&method, &path, &body);
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

// ------------------------------------------------------------ setup

fn demo_home(router: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let h = dir.path();
    let (code, _, err) = {
        let o = ost(h)
            .args(["init", "--me", "you@example.com"])
            .output()
            .unwrap();
        (
            o.status.code().unwrap(),
            String::new(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };
    assert_eq!(code, 0, "{err}");
    let files: Vec<String> = ["takeout.mbox", "Connections.csv", "contacts.vcf"]
        .iter()
        .map(|f| repo(&format!("fixtures/demo/{f}")).display().to_string())
        .collect();
    let mut args = vec!["import"];
    args.extend(files.iter().map(String::as_str));
    let o = ost(h).args(&args).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::write(
        h.join("config.toml"),
        format!(
            "[me]\nidentities = [\"email:you@example.com\"]\n\n[enrich]\nbackend = \"treg\"\n\n\
             [enrich.treg]\nendpoint = \"{router}\"\ntoken_env = \"OST_DEMO_TREG_TOKEN\"\n"
        ),
    )
    .unwrap();
    dir
}

/// A JSON-RPC session with `ost mcp`: the agent's quote-then-confirm flow.
fn agent_flow(home: &Path) -> (Value, Value) {
    let mut child = ost(home)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut req = |id: i64, method: &str, params: Value| -> Value {
        let msg = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        if id == 1 {
            writeln!(
                stdin,
                "{}",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .unwrap();
        }
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    };
    req(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},
               "clientInfo":{"name":"demo-agent","version":"0"}}),
    );
    let args = json!({"target":"Priya Sato","company":"Lumen Pay"});
    let quote = req(2, "tools/call", json!({"name":"path_to","arguments": args}))["result"].clone();
    let mut yes = args.clone();
    yes["allow_egress"] = json!(true);
    let done = req(3, "tools/call", json!({"name":"path_to","arguments": yes}))["result"].clone();
    drop(stdin);
    let _ = child.wait();
    (quote, done)
}

#[test]
fn phase_3_demo_on_the_demo_fixtures() {
    let (router, log) = start_router();
    let home = demo_home(&router);
    let h = home.path();
    let posts = |log: &Log| {
        log.lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "POST")
            .count()
    };

    println!("# 1. Not in my graph: plan and quote only, nothing sent");
    let (code, out, err) = show(
        h,
        "",
        &[
            "path",
            "Noor Haddad",
            "--company",
            "Lumen Pay",
            "--find-email",
        ],
    );
    assert_eq!(code, 3, "needs approval: {err}");
    assert!(out.contains("Total $0.0180"), "{out}");
    assert!(err.contains("re-run with --allow-egress"), "{err}");
    assert_eq!(posts(&log), 0, "nothing sent");

    println!("# 2. Run it");
    let (code, out, err) = show(
        h,
        "",
        &[
            "path",
            "Noor Haddad",
            "--company",
            "Lumen Pay",
            "--find-email",
            "--allow-egress",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("1. Theo Marsh (Lumen Pay)"), "{out}");
    assert!(out.contains("noor@lumenpay.example (deliverable)"), "{out}");
    assert_eq!(posts(&log), 4, "company, profile, find, verify");
    let (code, out, _) = show(h, "", &["enrich", "log", "--since", "1d"]);
    assert_eq!(code, 0);
    assert!(out.contains("3 requests sent, $0.0180 spent"), "{out}");
    assert!(!out.contains(TOKEN));

    println!("# 2b. Someone already in the graph: no egress at all");
    let before = posts(&log);
    let (code, out, _) = show(h, "", &["path", "Dana Whitfield", "--allow-egress"]);
    assert_eq!(code, 0);
    assert!(out.contains("No egress"), "{out}");
    assert_eq!(posts(&log), before);

    println!("# 3. Brief for the ask");
    let (code, out, _) = show(h, "", &["brief", "Maya Chen"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("# ") || out.contains("Maya Chen"), "{out}");
    assert!(out.lines().count() <= 12, "{out}");

    println!("# 4. Share a slice, not the graph");
    let colleague = tempfile::tempdir().unwrap();
    let c = colleague.path();
    let (code, key, _) = show(c, "(colleague) ", &["keys", "show"]);
    assert_eq!(code, 0);
    let key = key.trim().to_string();
    let (code, _, err) = show(
        h,
        "",
        &[
            "view",
            "create",
            "fintech-investors",
            "--query",
            r#"{"title":["Venture Partner","Principal"]}"#,
            "--ttl",
            "30d",
            "--to",
            &key,
        ],
    );
    assert_eq!(code, 0, "{err}");
    let file = h.join("fintech-investors.ost");
    let file_s = file.display().to_string();
    let (code, _, err) = show(
        h,
        "",
        &["view", "export", "fintech-investors", "-o", &file_s],
    );
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = show(c, "(colleague) ", &["view", "import", &file_s]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = show(
        c,
        "(colleague) ",
        &["view", "search", "fintech-investors", "Harbor Lane"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("Harbor Lane Capital"), "{out}");
    let (code, _, _) = show(h, "", &["view", "import", &file_s]);
    assert_eq!(
        code, 7,
        "the owner cannot read a file sent to the colleague"
    );

    println!("# 5. The same path_to flow from an agent (MCP)");
    let (quote, done) = agent_flow(h);
    println!(
        "agent -> path_to {{target: \"Priya Sato\", company: \"Lumen Pay\"}}\n{}",
        quote["content"][0]["text"].as_str().unwrap_or("")
    );
    println!(
        "agent -> (user agreed) path_to {{..., allow_egress: true}}\n{}",
        done["content"][0]["text"].as_str().unwrap_or("")
    );
    assert_eq!(quote["structuredContent"]["status"], "quote");
    assert_eq!(done["structuredContent"]["receipt"]["requests_sent"], 1);
}
