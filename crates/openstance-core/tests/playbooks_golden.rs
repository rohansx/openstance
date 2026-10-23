//! P4-CORE-01 golden: the Phase 1 fixtures (fixtures/EXPECTED.md, loaded
//! with fixtures/config.toml) at now = 2026-10-20T00:00:00Z give a fixed
//! `reconnect` list. Nobody in the fixtures is lapsed at the default
//! thresholds (F > 0.5 and R < 0.2), so the golden holds the default run
//! (empty) and a looser one (F > 0.1, R < 0.4) that ranks real rows.
//! Regenerate deliberately with `UPDATE_GOLDEN=1`.

use std::path::PathBuf;

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::playbook::{RunOptions, RunStatus};
use openstance_core::{config, FixedClock, Graph, Providers};
use serde_json::json;

/// 2026-10-20T00:00:00Z (playbooks.md section 11).
const NOW: i64 = 1_792_454_400;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/reconnect_fixtures.json"
);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

#[test]
fn reconnect_on_the_fixtures_matches_the_golden_file() {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g = Graph::open_in_memory(cfg, providers, Box::new(FixedClock(NOW))).unwrap();
    let files: Vec<PathBuf> = [
        "mbox/small.mbox",
        "linkedin/Connections.csv",
        "linkedin/messages.csv",
        "linkedin/Invitations.csv",
        "vcf/contacts.vcf",
    ]
    .iter()
    .map(|p| fixtures().join(p))
    .collect();
    g.import(&files, &ImportRequest::default()).unwrap();
    let mut both = serde_json::Map::new();
    for (key, input) in [
        ("default", json!({})),
        ("loose", json!({"min_frequency": 0.1, "max_recency": 0.4})),
    ] {
        let out = g
            .run_playbook("reconnect", &input, &RunOptions::default())
            .unwrap();
        assert_eq!(out.status, RunStatus::Done);
        assert_eq!(out.receipt.requests_sent, 0);
        both.insert(key.into(), out.result.unwrap());
    }
    assert!(!both["loose"]["people"].as_array().unwrap().is_empty());
    let got = serde_json::to_string_pretty(&both).unwrap() + "\n";
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, &got).unwrap();
    }
    let want = std::fs::read_to_string(GOLDEN).expect("run once with UPDATE_GOLDEN=1");
    assert_eq!(got, want);
}
