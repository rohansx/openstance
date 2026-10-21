//! P2-CLI-03 and P2-MODEL-02 end to end: `rebuild --embeddings`, embedding
//! during `import`, the "embedder changed" notice, and hybrid search JSON.
//! Uses the deterministic `hash:<dims>` embedder (no model, no network).

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const NOW: &str = "2026-10-20T12:00:00Z";
const CHANGED: &str = "embedder changed, run `openstance rebuild --embeddings`";

fn ost(home: &Path, embedder: &str) -> Command {
    let mut c = Command::cargo_bin("openstance").unwrap();
    c.env("OPENSTANCE_HOME", home)
        .env("HOME", home)
        .env_remove("OPENSTANCE_CONFIG")
        .env_remove("OPENSTANCE_DB")
        .env_remove("OPENSTANCE_LOG")
        .env("OPENSTANCE_OFFLINE", "1")
        .env("OPENSTANCE__MODEL__EMBEDDER", embedder)
        .env("NO_COLOR", "1")
        .args(["--now", NOW]);
    c
}

struct Out {
    stdout: String,
    stderr: String,
}

fn ok(c: &mut Command) -> Out {
    let out = c.output().unwrap();
    let o = Out {
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    };
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        o.stdout,
        o.stderr
    );
    o
}

fn json(c: &mut Command) -> Value {
    let o = ok(c);
    serde_json::from_str(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", o.stdout))
}

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    [
        "mbox/small.mbox",
        "linkedin/Connections.csv",
        "vcf/contacts.vcf",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect()
}

fn imported(embedder: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    ok(ost(dir.path(), embedder).args(["init", "--me", "owner@example.com"]));
    let mut c = ost(dir.path(), embedder);
    c.arg("import").args(fixtures());
    let o = ok(&mut c);
    assert!(
        o.stderr.is_empty() || !o.stderr.contains("error"),
        "{}",
        o.stderr
    );
    dir
}

#[test]
fn import_embeds_and_search_reports_both_halves() {
    let dir = imported("hash:384");
    let mut c = ost(dir.path(), "hash:384");
    c.arg("import").args(fixtures());
    // re-importing the same files changes nothing: no re-embedding
    let o = ok(&mut c);
    assert!(!o.stdout.contains(CHANGED) && !o.stderr.contains(CHANGED));

    let v = json(ost(dir.path(), "hash:384").args(["search", "engineering", "--json"]));
    assert_eq!(v["models"]["embedder"], "hash:384");
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");
    let results = v["results"].as_array().unwrap();
    assert!(!results.is_empty(), "{v}");
    let rel = &results[0]["relevance"];
    for k in ["fts", "vector", "fused", "final", "fts_rank", "vec_sim"] {
        assert!(rel.get(k).is_some(), "missing {k}: {rel}");
    }
    assert!(results.iter().any(|r| !r["relevance"]["vec_sim"].is_null()));
}

#[test]
fn rebuild_embeddings_reembeds_everyone_and_reports_it() {
    let dir = imported("hash:384");
    let v = json(ost(dir.path(), "hash:384").args(["rebuild", "--embeddings", "--json"]));
    assert_eq!(v["v"], 1);
    assert_eq!(v["embedder"], "hash:384");
    let n = v["embedded"].as_u64().unwrap();
    assert!(n > 5, "{v}");
    assert_eq!(v["unchanged"], 0);
    let human = ok(ost(dir.path(), "hash:384").args(["rebuild", "--embeddings"]));
    assert!(
        human
            .stdout
            .contains(&format!("embedded {n} people with hash:384")),
        "{}",
        human.stdout
    );
}

#[test]
fn changing_the_embedder_warns_until_rebuild_embeddings() {
    let dir = imported("hash:384");
    let o = ok(ost(dir.path(), "hash:64").args(["search", "engineering"]));
    assert!(o.stderr.contains(CHANGED), "{}", o.stderr);
    let v = json(ost(dir.path(), "hash:64").args(["search", "engineering", "--json"]));
    assert!(v["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["relevance"]["vector"].is_null()));
    // a plain rebuild does not switch embedders
    let o = ok(ost(dir.path(), "hash:64").arg("rebuild"));
    assert!(o.stderr.contains(CHANGED), "{}", o.stderr);
    let v = json(ost(dir.path(), "hash:64").args(["rebuild", "--embeddings", "--json"]));
    assert_eq!(v["recreated"], true);
    let o = ok(ost(dir.path(), "hash:64").args(["search", "engineering"]));
    assert!(!o.stderr.contains(CHANGED), "{}", o.stderr);
}

#[test]
fn rebuild_embeddings_with_no_embedder_is_a_clear_no_op() {
    let dir = imported("none");
    let o = ok(ost(dir.path(), "none").args(["rebuild", "--embeddings"]));
    assert!(
        o.stdout.contains("model.embedder = \"none\""),
        "{}",
        o.stdout
    );
}

#[test]
fn an_unavailable_model_fails_rebuild_embeddings_with_the_hint_but_not_import() {
    let dir = imported("fastembed:bge-small-en-v1.5");
    let out = ost(dir.path(), "fastembed:bge-small-en-v1.5")
        .args(["rebuild", "--embeddings"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("model.embedder = \\\"none\\\"") || err.contains("model.embedder = \"none\""),
        "{err}"
    );
    // search still works, FTS only
    let v = json(ost(dir.path(), "fastembed:bge-small-en-v1.5").args([
        "search",
        "engineering",
        "--json",
    ]));
    assert!(!v["results"].as_array().unwrap().is_empty());
}
