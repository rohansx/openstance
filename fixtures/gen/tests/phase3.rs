//! P3-IMP-01/02: the committed calendar and X archive fixtures equal the
//! generator, and loading them through the real pipeline on top of the
//! Phase 1 set gives the counts stated in `fixtures/EXPECTED.md`.

use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::{config, FixedClock, Graph, Providers};
use openstance_fixtures::{fixture_now, phase3};
use openstance_import::HandleKind;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn committed_binary_fixtures_match_the_generator() {
    for (rel, want) in phase3::binary_files() {
        let got = std::fs::read(fixtures().join(rel))
            .unwrap_or_else(|e| panic!("fixtures/{rel} missing ({e}); run gen-fixtures"));
        assert!(
            got == want,
            "fixtures/{rel} drifted from the generator; run gen-fixtures"
        );
    }
}

#[test]
fn generated_zip_is_readable() {
    let bytes = &phase3::binary_files()["x/twitter-archive.zip"];
    assert!(bytes.starts_with(b"PK\x03\x04"));
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.zip");
    std::fs::write(&p, bytes).unwrap();
    let imp = openstance_import::x_archive::XArchiveImporter::new(Default::default());
    let mut sink = openstance_import::MemorySink::default();
    use openstance_import::Importer;
    let stats = imp.import(&p, &mut sink).unwrap();
    let e = phase3::expected();
    assert_eq!(stats.records_read, e.follows + e.dms);
}

fn graph() -> Graph {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let text = text.replace(
        "identities = [",
        &format!("identities = [\"x_user_id:{}\", ", phase3::OWNER_X_ID),
    );
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let providers = Providers::with_importers(openstance_import::registry(&Default::default()));
    Graph::open_in_memory(cfg, providers, Box::new(FixedClock(fixture_now()))).unwrap()
}

#[test]
fn phase3_fixtures_on_top_of_phase1_resolve_as_expected() {
    let mut g = graph();
    let files: Vec<PathBuf> = [
        "mbox/small.mbox",
        "linkedin/Connections.csv",
        "linkedin/messages.csv",
        "linkedin/Invitations.csv",
        "vcf/contacts.vcf",
        "ics/google.ics",
        "ics/apple.ics",
        "x/twitter-archive.zip",
    ]
    .iter()
    .map(|p| fixtures().join(p))
    .collect();
    let resp = g.import(&files, &ImportRequest::default()).unwrap();
    let e = phase3::expected();

    let c = g.counts().unwrap();
    // Phase 1: 30 people. Calendars: Quinn Harlow, Saoirse Delacroix. X: three
    // id-only DM partners; follow-only ids stay dormant.
    assert_eq!(c.persons_excluding_self, 35);
    assert_eq!(c.flagged_people, 4, "flagged merges unchanged");
    assert_eq!(
        g.review().unwrap().suggestions.len(),
        2,
        "suggestions unchanged"
    );
    let meetings = e.google_meetings + e.apple_meetings - e.shared_meetings;
    assert_eq!(c.interactions, 193 + meetings + e.follows + e.dms);
    assert_eq!(c.identities, 44 + 2 + e.x_ids);
    let ics_dups: u64 = resp
        .sources
        .iter()
        .map(|s| s.stats.interactions_dup)
        .sum::<u64>();
    assert!(
        ics_dups >= e.shared_meetings,
        "the shared meeting is stored once"
    );

    // Meetings feed the ties: Priya and Marcus met, Marcus and Kofi share the standup.
    let priya = g.who(&PersonKey::Name("Priya Raman".into())).unwrap();
    assert!(
        priya
            .mutuals
            .iter()
            .any(|m| m.person.display_name.as_deref() == Some("Marcus Lee")
                && m.tie_kind == "meeting"),
        "{:?}",
        priya
            .mutuals
            .iter()
            .map(|m| (&m.person.display_name, &m.tie_kind))
            .collect::<Vec<_>>()
    );
    let quinn = g.who(&PersonKey::Name("Quinn Harlow".into())).unwrap();
    assert!(quinn.person.score > 0.0);

    // A follow-only X id is not a person; a DM partner is.
    assert!(g
        .who(&PersonKey::Handle(HandleKind::XUserId, "2000000002".into()))
        .is_err());
    assert!(g
        .who(&PersonKey::Handle(HandleKind::XUserId, "2000000009".into()))
        .is_ok());
}
