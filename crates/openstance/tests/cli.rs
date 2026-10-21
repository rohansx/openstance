//! End-to-end CLI tests: every command runs as a real process against a
//! temp `OPENSTANCE_HOME`. Graphs are seeded with the core test kit (direct
//! SQL) or imported from the synthetic fixtures.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use openstance_core::search::testkit;
use serde_json::Value;

/// `--now` for every seeded run (testkit::NOW = 2026-10-20T12:00:00Z).
const NOW: &str = "2026-10-20T12:00:00Z";

fn bin(name: &str, home: &Path) -> Command {
    let mut c = Command::cargo_bin(name).unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        // No model download in tests: the embedder tier lives in
        // tests/embeddings.rs (hash embedder) and the ignored model tests.
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("NO_COLOR", "1");
    c
}

fn ost(home: &Path) -> Command {
    bin("openstance", home)
}

fn stdout(c: &mut Command) -> String {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn json(c: &mut Command) -> Value {
    let s = stdout(c);
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn exit_code(c: &mut Command) -> i32 {
    c.output().unwrap().status.code().unwrap()
}

/// A home with `init` run and the test-kit graph seeded and indexed.
fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    stdout(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
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

fn result_ids(v: &Value) -> Vec<i64> {
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["person"]["id"].as_i64().unwrap())
        .collect()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

// --- P1-CLI-01 -------------------------------------------------------------

#[test]
fn help_lists_the_same_commands_for_both_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let a = stdout(ost(dir.path()).arg("--help"));
    let b = stdout(bin("ost", dir.path()).arg("--help"));
    assert_eq!(
        a.replace("Usage: openstance ", "Usage: X "),
        b.replace("Usage: ost ", "Usage: X ")
    );
    for cmd in [
        "init", "import", "rebuild", "review", "search", "who", "edit", "merge", "split", "export",
        "config",
    ] {
        assert!(a.contains(&format!("  {cmd}")), "{cmd} missing from:\n{a}");
    }
}

#[test]
fn usage_errors_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(exit_code(ost(dir.path()).arg("frobnicate")), 2);
    assert_eq!(
        exit_code(ost(dir.path()).args(["search", "--limit", "x"])),
        2
    );
}

// --- P1-CLI-02 -------------------------------------------------------------

#[test]
fn init_creates_home_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let first = stdout(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
    assert!(first.contains("created"), "{first}");
    assert!(dir.path().join("graph.db").exists());
    let cfg = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(cfg.contains("email:owner@example.com"), "{cfg}");
    let second = stdout(ost(dir.path()).args(["init"]));
    assert!(!second.contains("created"), "{second}");
    assert!(second.contains(&dir.path().join("graph.db").display().to_string()));
    let v = json(ost(dir.path()).args(["init", "--json"]));
    assert_eq!(v["created_db"], false);
    assert_eq!(v["schema_version"], 3);
}

// --- P1-CLI-07: config -----------------------------------------------------

#[test]
fn config_prints_effective_toml_with_origins() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[search]\ndefault_limit = 7\n",
    )
    .unwrap();
    let out = stdout(ost(dir.path()).arg("config"));
    assert!(out.contains("half_life_days = 180"), "{out}");
    assert!(out.contains("default_limit = 7  # file"), "{out}");
    assert!(out.contains("backend = \"none\""), "{out}");
    let get = stdout(ost(dir.path()).args(["config", "get", "search.default_limit"]));
    assert_eq!(get.trim(), "7");
    assert_eq!(
        exit_code(ost(dir.path()).args(["config", "get", "search.nope"])),
        5
    );
    let path = stdout(ost(dir.path()).args(["config", "path"]));
    assert!(path.contains("config.toml"), "{path}");
    let v = json(ost(dir.path()).args(["config", "--json"]));
    assert_eq!(v["config"]["search"]["default_limit"], 7);
    assert_eq!(v["origins"]["search.default_limit"], "file");
}

// --- P1-CLI-04: search -----------------------------------------------------

#[test]
fn search_json_and_table_agree() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args(["search", "--company", "contoso", "--json", "--now", NOW]));
    assert_eq!(v["v"], 1);
    assert_eq!(result_ids(&v), [3, 7]);
    assert_eq!(v["mode"], "structured");
    assert_eq!(v["results"][0]["evidence"].as_array().unwrap().len(), 3);
    assert!(v["results"][0]["person"]["components"]["recency"].is_number());

    let table = stdout(ost(h).args(["search", "--company", "contoso", "--now", NOW]));
    let marcus = table.find("Marcus Lee").expect("marcus");
    let ravi = table.find("Ravi Iyer").expect("ravi");
    assert!(marcus < ravi, "{table}");
    assert!(table.contains("Staff Engineer, Contoso"), "{table}");
    assert!(
        table.contains("\"devtools dinner\"  (work calendar)"),
        "{table}"
    );
    assert!(table.contains("2 results"), "{table}");
}

#[test]
fn explain_prints_the_parsed_query_and_a_rerun_that_reproduces_it() {
    let home = seeded();
    let h = home.path();
    let out = stdout(ost(h).args([
        "search",
        "founders at Northwind I've met",
        "--explain",
        "--now",
        NOW,
    ]));
    assert!(out.contains("query:"), "{out}");
    assert!(out.contains("title=[founder]"), "{out}");
    let rerun = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("rerun:"))
        .expect("rerun line")
        .trim()
        .to_string();
    assert_eq!(
        rerun,
        "openstance search --company northwind --title founder --met"
    );
    let parsed = json(ost(h).args([
        "search",
        "founders at Northwind I've met",
        "--json",
        "--now",
        NOW,
    ]));
    let mut args: Vec<String> = rerun
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect();
    args.extend(["--json".into(), "--now".into(), NOW.into()]);
    let again = json(ost(h).args(&args));
    assert_eq!(again["query"], parsed["query"]);
    assert_eq!(result_ids(&again), result_ids(&parsed));
    assert_eq!(result_ids(&again), [2]);
}

#[test]
fn search_flags_map_to_query_fields() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args([
        "search",
        "--title",
        "engineer",
        "--location",
        "berlin",
        "--since",
        "2025",
        "--met",
        "--min-score",
        "0.1",
        "--limit",
        "5",
        "--json",
        "--now",
        NOW,
    ]));
    let q = &v["query"];
    assert_eq!(q["title"][0], "engineer");
    assert_eq!(q["location"][0], "berlin");
    assert_eq!(q["since"], 1_735_689_600);
    assert_eq!(q["met_in_person"], true);
    assert_eq!(q["limit"], 5);
    assert_eq!(result_ids(&v), [3]);
    assert_eq!(
        v["query_source"]["explicit"],
        serde_json::json!([
            "title",
            "location",
            "since",
            "met_in_person",
            "min_score",
            "limit"
        ])
    );

    let bulk = json(ost(h).args(["search", "--include-bulk", "--json", "--now", NOW]));
    assert!(result_ids(&bulk).contains(&4));
    let lost = json(ost(h).args(["search", "--lost-touch", "--json", "--now", NOW]));
    assert_eq!(result_ids(&lost), [5]);
    let not_met = json(ost(h).args(["search", "--not-met", "--json", "--now", NOW]));
    assert_eq!(result_ids(&not_met), [5, 6]);
}

#[test]
fn text_search_with_evidence_and_no_parse() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args(["search", "devtools dinner", "--json", "--now", NOW]));
    assert_eq!(v["mode"], "text");
    assert_eq!(result_ids(&v)[0], 7);
    assert!(v["results"][0]["relevance"]["final"].is_number());
    let raw = json(ost(h).args(["search", "Board deck", "--no-parse", "--json", "--now", NOW]));
    assert_eq!(raw["query"]["text"], "Board deck");
    assert_eq!(raw["query_source"]["parser"], "passthrough");
}

#[test]
fn query_json_is_a_full_query() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args([
        "search",
        "--query-json",
        r#"{"company":["contoso"],"replied":true}"#,
        "--json",
        "--now",
        NOW,
    ]));
    assert_eq!(result_ids(&v), [7]);
    let mut c = ost(h);
    c.args(["search", "--query-json", "-", "--json", "--now", NOW])
        .write_stdin(r#"{"lost_touch": true}"#);
    assert_eq!(result_ids(&json(&mut c)), [5]);
    assert_eq!(
        exit_code(ost(h).args(["search", "--query-json", "{\"nope\":1}"])),
        2
    );
}

#[test]
fn bad_dates_are_usage_errors_with_json_error_body() {
    let home = seeded();
    let h = home.path();
    let out = ost(h)
        .args(["search", "--since", "Q5", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "bad_request");
}

// --- P1-CLI-07: export -----------------------------------------------------

#[test]
fn export_csv_and_json() {
    let home = seeded();
    let h = home.path();
    let csv = stdout(ost(h).args(["export", "--format", "csv", "--now", NOW]));
    let mut lines = csv.lines();
    assert!(lines
        .next()
        .unwrap()
        .starts_with("id,display_name,title,company"));
    assert_eq!(lines.count(), 5);

    let file = h.join("people.json");
    stdout(ost(h).args([
        "export",
        "--format",
        "json",
        "-o",
        file.to_str().unwrap(),
        "--now",
        NOW,
    ]));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["people"].as_array().unwrap().len(), 5);

    let some = stdout(ost(h).args([
        "export",
        "--format",
        "csv",
        "--company",
        "contoso",
        "--now",
        NOW,
    ]));
    assert_eq!(some.lines().count(), 3);
    let q = stdout(ost(h).args([
        "export",
        "--format",
        "csv",
        "--query",
        "at Contoso",
        "--now",
        NOW,
    ]));
    assert_eq!(q.lines().count(), 3);
    assert_eq!(exit_code(ost(h).args(["export", "--format", "xml"])), 2);
}

// --- who and person references ----------------------------------------------

#[test]
fn who_shows_a_person_and_missing_people_exit_5() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args(["who", "2", "--json", "--now", NOW]));
    assert_eq!(v["person"]["display_name"], "Priya Raman");
    let text = stdout(ost(h).args(["who", "2", "--now", NOW]));
    assert!(text.contains("Priya Raman"), "{text}");
    assert!(text.contains("score"), "{text}");
    assert_eq!(exit_code(ost(h).args(["who", "999999"])), 5);
}

// --- full pipeline on the fixtures (P1-CLI-03/05/06/07, demo script) --------

fn fixture_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures().join("config.toml"),
        dir.path().join("config.toml"),
    )
    .unwrap();
    stdout(ost(dir.path()).arg("init"));
    dir
}

fn import_all(h: &Path) -> String {
    let f = fixtures();
    stdout(ost(h).arg("import").args([
        f.join("mbox/small.mbox"),
        f.join("linkedin/Connections.csv"),
        f.join("linkedin/messages.csv"),
        f.join("linkedin/Invitations.csv"),
        f.join("vcf/contacts.vcf"),
    ]))
}

#[test]
fn import_prints_counts_and_is_idempotent() {
    let home = fixture_home();
    let h = home.path();
    let out = import_all(h);
    assert!(out.contains("30 people"), "{out}");
    assert!(out.contains("44 identities"), "{out}");
    assert!(out.contains("193 interactions"), "{out}");
    assert!(out.contains("4 flagged"), "{out}");
    let again = import_all(h);
    assert!(again.contains("already imported"), "{again}");
    assert!(again.contains("30 people"), "{again}");
}

#[test]
fn import_json_and_unknown_kind() {
    let home = fixture_home();
    let h = home.path();
    let v = json(
        ost(h)
            .args(["import", "--json"])
            .arg(fixtures().join("vcf/contacts.vcf")),
    );
    assert_eq!(v["v"], 1);
    assert_eq!(v["sources"][0]["kind"], "vcf");
    let out = ost(h)
        .args(["import", "--as", "nope"])
        .arg(fixtures().join("vcf/contacts.vcf"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("mbox") && err.contains("vcf"), "{err}");
}

#[test]
fn demo_edit_survives_rebuild_and_shows_in_search() {
    let home = fixture_home();
    let h = home.path();
    import_all(h);
    stdout(ost(h).args([
        "edit",
        "Priya Raman",
        "--title",
        "CTO",
        "--tag",
        "investor-intro",
    ]));
    stdout(ost(h).arg("rebuild"));
    let v = json(ost(h).args(["search", "Priya Raman", "--json"]));
    assert_eq!(v["results"][0]["person"]["title"], "CTO");
    assert_eq!(v["results"][0]["person"]["tags"][0], "investor-intro");
    let hist = stdout(ost(h).args(["edit", "Priya Raman", "--history"]));
    assert!(hist.contains("title"), "{hist}");
    let s = json(ost(h).args(["search", "--company", "Northwind", "--json"]));
    assert!(!result_ids(&s).is_empty());
    let digest = json(ost(h).args(["search", "digest", "--json"]));
    assert!(result_ids(&digest).is_empty(), "bulk sender hidden");
}

#[test]
fn ambiguous_names_list_candidates_and_exit_6() {
    let home = fixture_home();
    let h = home.path();
    import_all(h);
    let out = ost(h)
        .args(["edit", "Alex Kim", "--title", "x"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.matches("Alex Kim").count() >= 2, "{err}");
}

#[test]
fn review_lists_flagged_merges_and_confirm_writes_merged_into() {
    let home = fixture_home();
    let h = home.path();
    import_all(h);
    let q = json(ost(h).args(["review", "--json"]));
    let flagged = q["flagged"].as_array().unwrap();
    assert_eq!(flagged.len(), 4);
    let list = stdout(ost(h).args(["review", "--list"]));
    assert!(
        list.contains("name_company") || list.contains("name+company"),
        "{list}"
    );
    let id = flagged[0]["person"]["id"].as_i64().unwrap().to_string();
    stdout(ost(h).args(["review", "--confirm", &id]));
    stdout(ost(h).arg("rebuild"));
    let q = json(ost(h).args(["review", "--json"]));
    assert_eq!(q["flagged"].as_array().unwrap().len(), 3);
}

#[test]
fn merge_and_split_round_trip() {
    let home = fixture_home();
    let h = home.path();
    import_all(h);
    let who = |r: &str| json(ost(h).args(["who", r, "--json"]));
    let a = who("in/greta-solberg")["person"]["id"].as_i64().unwrap();
    let b = who("in/hugo-brandt")["person"]["id"].as_i64().unwrap();
    stdout(ost(h).args(["merge", &a.to_string(), &b.to_string()]));
    let merged = who(&a.to_string());
    assert_eq!(merged["identities"].as_array().unwrap().len(), 2);
    stdout(ost(h).args(["merge", "--undo", &a.to_string(), &b.to_string()]));
    let back = who(&a.to_string());
    assert_eq!(back["identities"].as_array().unwrap().len(), 1);

    let mira = who("mira@tailspin.example");
    assert_eq!(mira["identities"].as_array().unwrap().len(), 3);
    let mira_id = mira["person"]["id"].as_i64().unwrap().to_string();
    stdout(ost(h).args(["split", &mira_id, "--identity", "phone:+12015550188"]));
    let after = who("mira@tailspin.example");
    assert_eq!(after["identities"].as_array().unwrap().len(), 2);
    stdout(ost(h).arg("rebuild"));
    let after = who("mira@tailspin.example");
    assert_eq!(
        after["identities"].as_array().unwrap().len(),
        2,
        "split survives rebuild"
    );
}
