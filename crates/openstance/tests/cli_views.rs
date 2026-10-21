//! `openstance keys show`, `view create|list|show|export|import|search|rm`
//! and `export --format vcf` (P3-CLI-03, P3-CLI-05), end to end against
//! temp homes: an owner with the core test-kit graph and a recipient.

use std::path::Path;

use assert_cmd::Command;
use openstance_core::search::testkit;
use serde_json::Value;

const NOW: &str = "2026-10-20T12:00:00Z";
const IN_TWO_DAYS: &str = "2026-10-22T12:00:00Z";

fn ost(home: &Path) -> Command {
    let mut c = Command::cargo_bin("openstance").unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env("USER", "owen")
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("NO_COLOR", "1");
    c
}

fn run_ok(c: &mut Command) -> (String, String) {
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn json(c: &mut Command) -> Value {
    let (s, _) = run_ok(c);
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn exit_code(c: &mut Command) -> i32 {
    c.output().unwrap().status.code().unwrap()
}

fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run_ok(ost(dir.path()).args(["init", "--me", "owner@example.com"]));
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

fn recipient() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    run_ok(ost(dir.path()).arg("init"));
    let v = json(ost(dir.path()).args(["keys", "show", "--json"]));
    let pk = v["public_key"].as_str().unwrap().to_string();
    (dir, pk)
}

fn people_count(home: &Path) -> i64 {
    let conn = openstance_core::db::open(
        &home.join("graph.db"),
        &openstance_core::db::DbOptions {
            busy_timeout_ms: 5000,
        },
    )
    .unwrap();
    conn.query_row("SELECT count(*) FROM person", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn init_creates_the_age_key_and_keys_show_prints_it() {
    let dir = tempfile::tempdir().unwrap();
    run_ok(ost(dir.path()).arg("init"));
    let key = dir.path().join("age.key");
    assert!(key.exists(), "init generates the keypair");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let before = std::fs::read_to_string(&key).unwrap();
    let (out, _) = run_ok(ost(dir.path()).args(["keys", "show"]));
    assert!(out.trim().starts_with("age1"), "{out}");
    assert!(before.contains(out.trim()));
    // Never overwritten.
    run_ok(ost(dir.path()).arg("init"));
    run_ok(ost(dir.path()).args(["keys", "show"]));
    assert_eq!(std::fs::read_to_string(&key).unwrap(), before);
}

#[test]
fn keys_show_creates_a_key_on_first_need() {
    let dir = tempfile::tempdir().unwrap();
    let v = json(ost(dir.path()).args(["keys", "show", "--json"]));
    assert_eq!(v["v"], 1);
    assert!(v["public_key"].as_str().unwrap().starts_with("age1"));
    assert_eq!(v["created"], true);
}

#[test]
fn share_a_view_end_to_end() {
    let owner = seeded();
    let (rec, pk) = recipient();
    let out = owner.path().join("devtools.ost");

    let created = json(ost(owner.path()).args([
        "view",
        "create",
        "devtools",
        "--query",
        r#"{"company":["Northwind","Contoso"]}"#,
        "--ttl",
        "30d",
        "--to",
        &pk,
        "--now",
        NOW,
        "--json",
    ]));
    assert_eq!(created["name"], "devtools");
    assert_eq!(created["direction"], "own");
    assert_eq!(created["rows"], 3);
    assert_eq!(created["expires_at"], "2026-11-19T12:00:00Z");
    assert_eq!(created["query"]["company"][1], "Contoso");

    let (_, err) = run_ok(ost(owner.path()).args([
        "view",
        "export",
        "devtools",
        "-o",
        out.to_str().unwrap(),
        "--now",
        NOW,
    ]));
    assert!(err.contains("wrote 3 people"), "{err}");
    assert!(err.contains("advisory"), "{err}");
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"age-encryption.org/v1"));

    let own = json(ost(owner.path()).args(["view", "list", "--json", "--now", NOW]));
    assert_eq!(own["views"].as_array().unwrap().len(), 1);
    assert_eq!(own["views"][0]["direction"], "own");
    assert!(own["views"][0]["id"].as_str().unwrap().len() == 26);

    let before = people_count(rec.path());
    let imported = json(ost(rec.path()).args([
        "view",
        "import",
        out.to_str().unwrap(),
        "--now",
        NOW,
        "--json",
    ]));
    assert_eq!(imported["status"], "imported");
    assert_eq!(imported["view"]["owner"]["label"], "owen");
    assert_eq!(imported["view"]["rows"], 3);
    assert_eq!(
        people_count(rec.path()),
        before,
        "never merged into graph.db"
    );
    let stored = rec.path().join("views");
    assert_eq!(std::fs::read_dir(&stored).unwrap().count(), 1);

    let hits = json(ost(rec.path()).args([
        "view",
        "search",
        "devtools",
        "northwind",
        "--now",
        NOW,
        "--json",
    ]));
    let results = hits["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["person"]["display_name"], "Priya Raman");
    assert_eq!(results[0]["person"]["score_bucket"], "strong");
    assert_eq!(results[0]["person"]["emails"][0], "priya@northwind.example");

    let (table, _) = run_ok(ost(rec.path()).args(["view", "search", "*", "--now", NOW]));
    assert!(table.contains("Marcus Lee"), "{table}");

    let list = json(ost(rec.path()).args([
        "view",
        "list",
        "--direction",
        "imported",
        "--now",
        NOW,
        "--json",
    ]));
    assert_eq!(list["views"].as_array().unwrap().len(), 1);
    assert_eq!(list["views"][0]["direction"], "imported");

    // The owner cannot read a file encrypted only to the recipient: exit 7.
    assert_eq!(
        exit_code(ost(owner.path()).args(["view", "import", out.to_str().unwrap(), "--now", NOW])),
        7
    );

    run_ok(ost(rec.path()).args(["view", "rm", "devtools", "--now", NOW]));
    assert_eq!(std::fs::read_dir(&stored).unwrap().count(), 0);
}

#[test]
fn expired_views_are_refused_on_import_with_exit_7() {
    let owner = seeded();
    let (rec, pk) = recipient();
    run_ok(ost(owner.path()).args([
        "view",
        "create",
        "short",
        "--query",
        "northwind",
        "--ttl",
        "1d",
        "--to",
        &pk,
        "--now",
        NOW,
    ]));
    let (bytes, _) = {
        let out = ost(owner.path())
            .args(["view", "export", "short", "--now", NOW])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        (out.stdout, ())
    };
    let file = rec.path().join("short.ost");
    std::fs::write(&file, &bytes).unwrap();
    let out = ost(rec.path())
        .args([
            "view",
            "import",
            file.to_str().unwrap(),
            "--now",
            IN_TWO_DAYS,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert!(String::from_utf8_lossy(&out.stderr).contains("expired"));
}

#[test]
fn ttl_never_needs_yes_and_existing_views_need_yes_to_replace() {
    let owner = seeded();
    assert_eq!(
        exit_code(ost(owner.path()).args(["view", "create", "x", "--ttl", "never"])),
        2
    );
    run_ok(ost(owner.path()).args(["view", "create", "x", "--ttl", "never", "--yes"]));
    assert_eq!(
        exit_code(ost(owner.path()).args(["view", "create", "x", "--company", "Contoso"])),
        2
    );
    let v = json(ost(owner.path()).args([
        "view",
        "create",
        "x",
        "--company",
        "Contoso",
        "--yes",
        "--json",
    ]));
    assert_eq!(v["query"]["company"][0], "Contoso");
    let shown = json(ost(owner.path()).args(["view", "show", "x", "--json"]));
    assert_eq!(shown["name"], "x");
    assert_eq!(shown["recipients"].as_array().map_or(0, Vec::len), 0);
}

#[test]
fn export_format_vcf_writes_vcards() {
    let owner = seeded();
    let (out, err) = run_ok(ost(owner.path()).args(["export", "--format", "vcf", "--now", NOW]));
    assert_eq!(out.matches("BEGIN:VCARD\r\n").count(), 5, "{out}");
    assert!(out.contains("FN:Priya Raman\r\n"));
    assert!(err.contains("exported 5 people"), "{err}");
}
