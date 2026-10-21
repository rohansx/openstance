//! Shared helpers for the importer integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use openstance_import::*;

/// The sentinel planted in every body and free-text field of the fixtures.
pub const SENTINEL: &str = "SENTINEL-BODY-7f3a";

/// `fixtures/<rel>` at the workspace root.
pub fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

/// Write `content` to `name` inside a fresh temp dir.
pub fn temp_file(name: &str, content: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    std::fs::write(&p, content).unwrap();
    (dir, p)
}

/// Run an importer into a fresh `MemorySink`.
pub fn run(imp: &dyn Importer, path: &Path) -> (ImportStats, MemorySink) {
    let mut sink = MemorySink::default();
    let stats = imp.import(path, &mut sink).expect("import succeeds");
    (stats, sink)
}

fn push_hints(out: &mut Vec<String>, h: &Hints) {
    for v in [&h.name, &h.title, &h.company, &h.location]
        .into_iter()
        .flatten()
    {
        out.push(v.clone());
    }
}

/// Every string a sink received (handles, hints, subjects, ids, locators),
/// unredacted, for sentinel checks.
pub fn all_strings(sink: &MemorySink) -> Vec<String> {
    let mut out = Vec::new();
    for i in &sink.identities {
        out.push(i.handle.raw.clone());
        out.push(i.record.clone());
        push_hints(&mut out, &i.hints);
    }
    for i in &sink.interactions {
        out.extend(i.subject.clone());
        out.extend(i.external_id.clone());
        out.extend(i.thread_key.clone());
        for p in &i.participants {
            for h in &p.handles {
                out.push(h.raw.clone());
            }
            push_hints(&mut out, &p.hints);
        }
    }
    for (_, loc) in &sink.skips {
        out.push(loc.clone());
    }
    out
}

/// Assert no stored string carries the body sentinel.
pub fn assert_no_sentinel(sink: &MemorySink) {
    for s in all_strings(sink) {
        assert!(!s.contains(SENTINEL), "sentinel leaked into a record");
    }
}

/// Handles of one kind, raw, in emission order (identities then participants).
pub fn handles(sink: &MemorySink, kind: HandleKind) -> Vec<String> {
    let mut out: Vec<String> = sink
        .identities
        .iter()
        .filter(|i| i.handle.kind == kind)
        .map(|i| i.handle.raw.clone())
        .collect();
    for i in &sink.interactions {
        for p in &i.participants {
            for h in &p.handles {
                if h.kind == kind {
                    out.push(h.raw.clone());
                }
            }
        }
    }
    out
}

/// `chrono` helper: unix seconds for a UTC date-time.
pub fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(y, mo, d, h, mi, s)
        .unwrap()
        .timestamp()
}
