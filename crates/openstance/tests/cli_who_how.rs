//! P2-CLI-01 `who` and P2-CLI-02 `how` / `how-connected`, end to end
//! against a temp `OPENSTANCE_HOME` seeded with the core test kit.

use std::path::Path;

use assert_cmd::Command;
use openstance_core::graphq::{self, HowOptions};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::person::DetailOptions;
use openstance_core::search::testkit;
use openstance_core::Config;
use serde_json::Value;

const NOW: &str = "2026-10-20T12:00:00Z";

fn ost(home: &Path) -> Command {
    let mut c = Command::cargo_bin("openstance").unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        // Offline: no model download (edit/import re-embed with the
        // configured embedder).
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("NO_COLOR", "1");
    c
}

fn run(c: &mut Command) -> (i32, String) {
    let out = c.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

fn stdout(c: &mut Command) -> String {
    let (code, s) = run(c);
    assert_eq!(code, 0, "stdout: {s}");
    s
}

fn json(c: &mut Command) -> Value {
    let s = stdout(c);
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

/// The home's database (a `rusqlite::Connection`; a macro so this crate
/// needs no direct rusqlite dependency).
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

/// `init`, the test-kit graph, and ties around Marcus (3):
/// Priya (2) shares "Board deck draft" with him, Ravi (7) a meeting.
fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    stdout(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
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

fn graph(home: &Path) -> openstance_core::Graph {
    openstance_core::Graph::open(
        Config::default(),
        &home.join("graph.db"),
        openstance_core::Providers::null(),
        Box::new(openstance_core::FixedClock(testkit::NOW)),
    )
    .unwrap()
}

// --- who ---------------------------------------------------------------------

#[test]
fn who_json_is_the_get_person_shape() {
    let home = seeded();
    let h = home.path();
    let cli = json(ost(h).args(["who", "marcus@contoso.example", "--json", "--now", NOW]));
    let core = graphq::get_person(
        graph(h).conn(),
        &Config::default(),
        &PersonKey::parse("marcus@contoso.example"),
        DetailOptions::default(),
        testkit::NOW,
    )
    .unwrap();
    assert_eq!(cli, serde_json::to_value(core).unwrap());
    assert_eq!(cli["v"], 1);
}

#[test]
fn who_explains_the_score_component_by_component() {
    let home = seeded();
    let h = home.path();
    let text = stdout(ost(h).args(["who", "2", "--now", NOW]));
    // Priya: 0.35*0.95 + 0.20*0.9 + 0.25*0.7 + 0.15*0.6 + 0.05*0.5 = 0.8025
    // (the seeded stored score is 0.80; real scoring makes them agree).
    assert!(text.contains("score 0.80"), "{text}");
    for line in [
        "recency      0.950 x 0.35 = 0.332",
        "frequency    0.900 x 0.20 = 0.180",
        "reciprocity  0.700 x 0.25 = 0.175",
        "meetings     0.600 x 0.15 = 0.090",
        "breadth      0.500 x 0.05 = 0.025",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(text.contains("sum                       = 0.802"), "{text}");
    assert!(text.contains("email:priya@northwind.example"), "{text}");
    assert!(text.contains("\"Re: seed round notes\""), "{text}");
    assert!(text.contains("Marcus Lee"), "mutuals: {text}");
}

#[test]
fn who_shows_do_not_share_and_field_provenance() {
    let home = seeded();
    let h = home.path();
    stdout(ost(h).args([
        "edit",
        "3",
        "--no-share",
        "--title",
        "Principal",
        "--now",
        NOW,
    ]));
    let text = stdout(ost(h).args(["who", "3", "--now", NOW]));
    assert!(text.contains("do not share"), "{text}");
    assert!(text.contains("title: override"), "{text}");
    let v = json(ost(h).args(["who", "3", "--json", "--now", NOW]));
    assert_eq!(v["person"]["do_not_share"], true);
}

#[test]
fn who_unknown_is_exit_5() {
    let home = seeded();
    assert_eq!(run(ost(home.path()).args(["who", "999999"])).0, 5);
}

// --- how ---------------------------------------------------------------------

#[test]
fn how_json_is_identical_to_the_core_and_mcp_shape() {
    let home = seeded();
    let h = home.path();
    let cli = json(ost(h).args(["how", "Marcus Lee", "--json", "--now", NOW]));
    let core = graphq::how_connected(
        graph(h).conn(),
        &PersonKey::parse("Marcus Lee"),
        HowOptions::default(),
        testkit::NOW,
    )
    .unwrap();
    assert_eq!(cli, serde_json::to_value(core).unwrap());
    assert_eq!(cli["paths"][0]["via"]["display_name"], "Priya Raman");
    assert_eq!(cli["paths"][1]["via"]["display_name"], "Ravi Iyer");
}

#[test]
fn how_connected_is_an_alias_and_limit_applies() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args(["how-connected", "3", "--limit", "1", "--json", "--now", NOW]));
    assert_eq!(v["paths"].as_array().unwrap().len(), 1);
}

#[test]
fn how_prints_direct_then_paths_with_strength_and_evidence() {
    let home = seeded();
    let h = home.path();
    let text = stdout(ost(h).args(["how", "Marcus Lee", "--now", NOW]));
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("direct: score 0.50, last contact 2026-09-30"),
        "{text}"
    );
    // Priya 0.8 x 0.4 = 0.32; Ravi 0.45 x 0.5 = 0.225.
    assert!(
        text.contains("you -> Priya Raman (score 0.80) -> Marcus Lee (tie 0.40)"),
        "{text}"
    );
    assert!(text.contains("strength 0.320"), "{text}");
    assert!(text.contains("\"Board deck draft\""), "{text}");
    assert!(
        text.contains("you and Priya Raman: "),
        "my last interaction with the mutual: {text}"
    );
    assert!(text.find("Priya Raman").unwrap() < text.find("Ravi Iyer").unwrap());
}

#[test]
fn how_to_someone_outside_the_graph_points_at_path_to() {
    let home = seeded();
    let h = home.path();
    let v = json(ost(h).args(["how", "kai@ramp.example", "--json", "--now", NOW]));
    assert_eq!(
        v,
        serde_json::json!({ "v": 1, "found": false,
            "hint": "path_to (Phase 3) can look outside your graph" })
    );
    let text = stdout(ost(h).args(["how", "Kai Wu", "--now", NOW]));
    assert!(text.contains("not in your graph"), "{text}");
    assert!(text.contains("path_to"), "{text}");
}

#[test]
fn how_without_paths_says_so_and_errors_map_to_exit_codes() {
    let home = seeded();
    let h = home.path();
    let text = stdout(ost(h).args(["how", "Sam Ito", "--now", NOW]));
    assert!(text.contains("no warm paths"), "{text}");
    // The owner is not a target.
    assert_eq!(run(ost(h).args(["how", "owner@example.com"])).0, 2);
    let conn = db!(h);
    conn.execute_batch("INSERT INTO person(id, display_name) VALUES (90, 'Marcus Lee')")
        .unwrap();
    let (code, out) = run(ost(h).args(["how", "Marcus Lee", "--json"]));
    assert_eq!(code, 6);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "ambiguous");
}
