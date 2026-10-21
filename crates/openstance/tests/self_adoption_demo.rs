//! The demo with a plain `openstance init` (no `--me`), as the UI e2e and a
//! first-time user run it: the mailbox owner (Jordan Avery,
//! you@example.com) is adopted as "me" from the Sent mail, the import says
//! so and how to change it, and the owner never appears as a contact, the
//! best mutual in a brief, or a hop in an intro path.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const NOW: &str = "2026-10-15";

fn demo(f: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/demo")
        .join(f)
}

fn ost(home: &Path) -> Command {
    let mut c = Command::cargo_bin("openstance").unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("OPENSTANCE__MODEL__EMBEDDER", "none")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("NO_COLOR", "1");
    c
}

fn ok(c: &mut Command) -> String {
    let out = c.output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "stdout: {s}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    s
}

fn imported_demo(json: bool) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    ok(ost(dir.path()).arg("init"));
    let mut c = ost(dir.path());
    c.args(["--now", NOW, "import"]);
    if json {
        c.arg("--json");
    }
    for f in ["takeout.mbox", "Connections.csv", "contacts.vcf"] {
        c.arg(demo(f));
    }
    let out = ok(&mut c);
    (dir, out)
}

#[test]
fn import_reports_the_adopted_owner_and_how_to_change_it() {
    let (dir, out) = imported_demo(false);
    assert!(
        out.contains("self          treating you@example.com as you"),
        "{out}"
    );
    assert!(out.contains("[me] identities"), "{out}");
    assert!(
        out.contains(&dir.path().join("config.toml").display().to_string()),
        "names the config file: {out}"
    );

    let (_dir, out) = imported_demo(true);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sources"][0]["self_adopted"][0], "email:you@example.com");
    assert!(v["sources"][1].get("self_adopted").is_none(), "{v}");
}

#[test]
fn the_owner_is_not_a_contact_mutual_or_hop() {
    let (dir, _) = imported_demo(false);
    let h = dir.path();
    let found: Value = serde_json::from_str(&ok(ost(h).args([
        "--now",
        NOW,
        "search",
        "Jordan Avery",
        "--json",
        "--no-parse",
    ])))
    .unwrap();
    let names: Vec<&str> = found["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["person"]["display_name"].as_str())
        .collect();
    assert!(!names.contains(&"Jordan Avery"), "{names:?}");

    let brief = ok(ost(h).args(["--now", NOW, "brief", "Maya Chen"]));
    assert!(brief.contains("Best mutual:"), "{brief}");
    assert!(!brief.contains("Jordan Avery"), "{brief}");

    let how = ok(ost(h).args(["--now", NOW, "how", "Maya Chen"]));
    assert!(!how.contains("Jordan Avery"), "{how}");
}
