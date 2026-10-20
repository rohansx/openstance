//! P1-CORE-12: importing every Phase 1 fixture yields exactly the totals in
//! `fixtures/EXPECTED.md` (generated from `fixtures/gen`).
//!
//! Ignored until the resolve, score and import streams land their
//! implementations; the resolve stream removes the `#[ignore]`.

use std::path::PathBuf;

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::{config, FixedClock, Graph, Providers};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// 2026-10-01T00:00:00Z, the fixtures' "today".
const FIXTURE_NOW: i64 = 1_790_812_800;

fn fixture_files() -> Vec<PathBuf> {
    [
        "mbox/small.mbox",
        "linkedin/Connections.csv",
        "linkedin/messages.csv",
        "linkedin/Invitations.csv",
        "vcf/contacts.vcf",
    ]
    .iter()
    .map(|p| fixtures().join(p))
    .collect()
}

fn fixture_graph() -> Graph {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let (cfg, warnings) = config::parse_str(&text, true).unwrap();
    assert!(warnings.is_empty());
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    Graph::open_in_memory(cfg, providers, Box::new(FixedClock(FIXTURE_NOW))).unwrap()
}

#[test]
fn fixture_config_loads_strictly() {
    let g = fixture_graph();
    assert_eq!(g.config().me.identities.len(), 2);
    assert_eq!(g.config().resolution.gmail_domains[0], "gmail.test");
}

#[test]
#[ignore = "enable when ingest + resolve + score + importers land (P1-CORE-12)"]
fn fixture_totals_match_expected_md() {
    let mut g = fixture_graph();
    g.import(&fixture_files(), &ImportRequest::default())
        .unwrap();
    let c = g.counts().unwrap();
    // Keep in sync with fixtures/EXPECTED.md.
    assert_eq!(c.persons, 31, "persons including self");
    assert_eq!(c.persons_excluding_self, 30);
    assert_eq!(c.flagged_people, 4, "flagged merges");
    assert_eq!(c.bulk, 1, "bulk senders");
    assert_eq!(c.identities, 44);
    assert_eq!(c.interactions, 193, "174 email + 14 connection + 5 dm");
    assert_eq!(c.contacts, 11);
    assert_eq!(g.review().unwrap().suggestions.len(), 2);
}

#[test]
#[ignore = "enable when ingest + resolve land (P1-CORE-05)"]
fn reimporting_the_fixtures_is_a_no_op() {
    let mut g = fixture_graph();
    g.import(&fixture_files(), &ImportRequest::default())
        .unwrap();
    let before = g.counts().unwrap();
    let again = g
        .import(&fixture_files(), &ImportRequest::default())
        .unwrap();
    assert!(again.sources.iter().all(|s| s.skipped_duplicate_file));
    assert_eq!(g.counts().unwrap(), before);
}
