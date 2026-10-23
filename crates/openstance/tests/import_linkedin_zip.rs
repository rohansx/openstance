//! The LinkedIn export `.zip` through the CLI: `openstance import <zip>`
//! auto-detects it, imports the three CSVs as one source labelled with the
//! zip's export date, and rejects LinkedIn's Basic export (no
//! Connections.csv) with an actionable message.

use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const BASIC: &str = "This is LinkedIn's Basic export; it has no Connections.csv. Request the \
                     larger archive (Settings > Data privacy > Get a copy of your data > \
                     'Download larger data archive').";

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

fn home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let out = ost(dir.path())
        .args(["init", "--me", "owner@example.com"])
        .output()
        .unwrap();
    assert!(out.status.success());
    dir
}

fn zip_entries(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (n, b) in entries {
        z.start_file(*n, o).unwrap();
        z.write_all(b).unwrap();
    }
    z.finish().unwrap();
}

fn csv(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join("linkedin").join(name)).unwrap()
}

#[test]
fn complete_export_zip_imports_as_one_linkedin_source() {
    let h = home();
    let zip = h
        .path()
        .join("Complete_LinkedInDataExport_10-20-2026.zip.zip");
    zip_entries(
        &zip,
        &[
            ("Connections.csv", csv("Connections.csv")),
            ("messages.csv", csv("messages.csv")),
            ("Invitations.csv", csv("Invitations.csv")),
            (
                "Profile.csv",
                b"First Name,Last Name\nOwen,Ryder\n".to_vec(),
            ),
        ],
    );
    let out = ost(h.path())
        .args(["import", "--json"])
        .arg(&zip)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{s}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["sources"].as_array().unwrap().len(), 1, "{v}");
    let src = &v["sources"][0];
    assert_eq!(src["kind"], "linkedin_csv");
    assert_eq!(src["label"], "linkedin 2026-10-20");
    // 12 connections + 8 messages kept + 2 invitations, as when the three
    // CSVs are imported one by one.
    let mut one_by_one = 0;
    let h2 = home();
    for f in ["Connections.csv", "messages.csv", "Invitations.csv"] {
        let out = ost(h2.path())
            .args(["import", "--json"])
            .arg(fixtures().join("linkedin").join(f))
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        one_by_one += v["sources"][0]["interactions"].as_u64().unwrap();
    }
    assert_eq!(src["interactions"].as_u64().unwrap(), one_by_one, "{v}");
}

#[test]
fn basic_export_zip_fails_with_the_fix() {
    let h = home();
    let zip = h.path().join("Basic_LinkedInDataExport_10-20-2026.zip");
    zip_entries(
        &zip,
        &[
            (
                "Profile.csv",
                b"First Name,Last Name\nOwen,Ryder\n".to_vec(),
            ),
            ("Registration.csv", b"Registered At\n2020-01-01\n".to_vec()),
        ],
    );
    let out = ost(h.path()).arg("import").arg(&zip).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a usage error, not a crash: {err}"
    );
    assert!(err.contains(BASIC), "{err}");

    let out = ost(h.path())
        .args(["import", "--json"])
        .arg(&zip)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("\"bad_request\""), "{all}");
    assert!(all.contains("Basic export"), "{all}");
}
