//! P3-IMP-01/02 through the CLI: `openstance import` detects calendar
//! exports and X archives, and passes `[me]` and `[import]
//! ics_max_attendees` to the ics importer.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
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

fn json(c: &mut Command) -> Value {
    let out = c.output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "stdout: {s}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let out = ost(dir.path())
        .args(["init", "--me", "owner@example.com"])
        .output()
        .unwrap();
    assert!(out.status.success());
    dir
}

#[test]
fn imports_calendar_and_x_archive() {
    let h = home();
    let v = json(
        ost(h.path())
            .args(["import", "--json"])
            .arg(fixtures().join("ics/google.ics"))
            .arg(fixtures().join("x/twitter-archive.zip")),
    );
    assert_eq!(v["sources"][0]["kind"], "ics");
    assert_eq!(v["sources"][0]["interactions"], 117);
    assert_eq!(v["sources"][1]["kind"], "x_archive");
    assert_eq!(v["sources"][1]["interactions"], 21);
}

#[test]
fn owner_from_me_skips_declined_events() {
    let h = home();
    let ics = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Work\r\n\
BEGIN:VEVENT\r\nUID:declined@test\r\nDTSTART:20260803T140000Z\r\n\
ORGANIZER:mailto:nora@fabrikam.example\r\n\
ATTENDEE;PARTSTAT=DECLINED:mailto:owner@example.com\r\n\
ATTENDEE;PARTSTAT=ACCEPTED:mailto:priya@northwind.example\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:accepted@test\r\nDTSTART:20260804T140000Z\r\n\
ORGANIZER:mailto:nora@fabrikam.example\r\n\
ATTENDEE;PARTSTAT=ACCEPTED:mailto:owner@example.com\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";
    let p = h.path().join("work.ics");
    std::fs::write(&p, ics).unwrap();
    let v = json(ost(h.path()).args(["import", "--json"]).arg(&p));
    assert_eq!(v["sources"][0]["interactions"], 1, "{v}");
}
