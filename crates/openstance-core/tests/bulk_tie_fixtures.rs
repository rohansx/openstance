//! P1-CORE-10 / P1-CORE-13 acceptance on the real Phase 1 fixtures
//! (`fixtures/EXPECTED.md`): Digest Weekly is the only bulk sender, Priya
//! (busy two-way) is not, Priya and Marcus share exactly 3 email
//! interactions, and the self person never appears in `tie`.
//!
//! Runs ingest + resolve + the importers end to end.

use std::path::PathBuf;

use openstance_core::config::ScoreSection;
use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::score::{ties, ScoreConfig};
use openstance_core::{config, FixedClock, Graph, Providers};
use rusqlite::OptionalExtension;

/// 2026-10-01T00:00:00Z, the fixtures' "today".
const FIXTURE_NOW: i64 = 1_790_812_800;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn imported() -> Graph {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g = Graph::open_in_memory(cfg, providers, Box::new(FixedClock(FIXTURE_NOW))).unwrap();
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
    g
}

fn person_by_email(g: &Graph, email: &str) -> i64 {
    g.conn()
        .query_row(
            "SELECT person_id FROM identity WHERE kind = 'email' AND value = ?1",
            [email],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
        .unwrap_or_else(|| panic!("no person for {email}"))
}

#[test]
fn bulk_fixture_digest_is_bulk_and_priya_is_not() {
    let g = imported();
    let digest = person_by_email(&g, "news@digest.example");
    let priya = person_by_email(&g, "priya@northwind.example");
    let bulk: Vec<i64> = g
        .conn()
        .prepare("SELECT id FROM person WHERE is_bulk = 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(bulk, vec![digest]);
    let (score, is_bulk): (f64, i64) = g
        .conn()
        .query_row(
            "SELECT score, is_bulk FROM person WHERE id = ?1",
            [priya],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(is_bulk, 0);
    assert!(score > 0.0);
}

#[test]
fn tie_fixture_priya_marcus_share_three_emails() {
    let g = imported();
    let priya = person_by_email(&g, "priya@northwind.example");
    let marcus = person_by_email(&g, "marcus@contoso.example");
    let (a, b) = (priya.min(marcus), priya.max(marcus));
    let n: i64 = g
        .conn()
        .query_row(
            "SELECT n_shared FROM tie WHERE a = ?1 AND b = ?2 AND kind = 'email'",
            [a, b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 3);
    let ev = ties::evidence(g.conn(), priya, marcus, 10).unwrap();
    assert_eq!(ev.len(), 3);
    assert!(ev.windows(2).all(|w| w[0].at >= w[1].at));

    let self_in_tie: i64 = g
        .conn()
        .query_row(
            "SELECT count(*) FROM tie WHERE a IN (SELECT id FROM person WHERE is_self = 1)
                                     OR b IN (SELECT id FROM person WHERE is_self = 1)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(self_in_tie, 0);
    let bad: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM tie WHERE a >= b", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bad, 0);
    let _ = ScoreConfig::from_config(&ScoreSection::default()).unwrap();
}

mod score_support;

/// The same acceptance checks without ingest/resolve: the real mbox
/// importer feeds a MemorySink, and each distinct (lowercased) email
/// address becomes one person, which is all the bulk and tie rules need.
#[test]
fn bulk_tie_fixture_mbox_with_per_address_people() {
    use std::collections::{BTreeMap, BTreeSet};

    use openstance_core::import::{HandleKind, Importer, MemorySink};
    use openstance_core::score;

    let mut sink = MemorySink::default();
    openstance_import::mbox::MboxImporter::new(ImportOptions::default())
        .import(&fixtures().join("mbox/small.mbox"), &mut sink)
        .unwrap();

    let mut s = score_support::Seed::new();
    let owner = s.owner("owner@example.com");
    let mut people: BTreeMap<String, i64> = BTreeMap::new();
    people.insert("owner@example.com".into(), owner);
    let mut seen_ext = BTreeSet::new();
    for i in &sink.interactions {
        if let Some(ext) = &i.external_id {
            if !seen_ext.insert(ext.clone()) {
                continue; // global dedupe, as the Sink does
            }
        }
        let mut parts: Vec<(i64, &str)> = Vec::new();
        for p in &i.participants {
            for h in &p.handles {
                if h.kind != HandleKind::Email {
                    continue;
                }
                let email = h.raw.trim().to_lowercase();
                let id = match people.get(&email) {
                    Some(id) => *id,
                    None => {
                        let id = s.person(&email);
                        people.insert(email, id);
                        id
                    }
                };
                if !parts.contains(&(id, p.role.as_str())) {
                    parts.push((id, p.role.as_str()));
                }
            }
        }
        let owner_from = parts.iter().any(|(id, r)| *id == owner && *r == "from");
        let owner_in = parts.iter().any(|(id, _)| *id == owner);
        let dir = if owner_from {
            "out"
        } else if owner_in {
            "in"
        } else {
            i.dir_hint.map_or("none", |d| d.as_str())
        };
        s.interaction(
            "mbox",
            i.kind.as_str(),
            i.at,
            dir,
            i.thread_key.as_deref(),
            i.list_unsub,
            i.subject.as_deref(),
            &parts,
        );
    }

    let report = score::recompute_all(
        &s.conn,
        &ScoreConfig::from_config(&ScoreSection::default())
            .unwrap()
            .0,
        FIXTURE_NOW,
    )
    .unwrap();
    let digest = people["news@digest.example"];
    let priya = people["priya@northwind.example"];
    let marcus = people["marcus@contoso.example"];
    assert_eq!(report.bulk, 1, "Digest Weekly is the only bulk sender");
    assert!(s.is_bulk(digest));
    assert!(!s.is_bulk(priya));
    assert!(s.stored(priya)[0] > 0.0);

    let rows = s.ties();
    let pm: Vec<_> = rows
        .iter()
        .filter(|t| (t.0, t.1) == (priya.min(marcus), priya.max(marcus)) && t.2 == "email")
        .collect();
    assert_eq!(pm.len(), 1);
    assert_eq!(pm[0].5, 3, "n_shared");
    assert_eq!(
        score::ties::evidence(&s.conn, priya, marcus, 10)
            .unwrap()
            .len(),
        3
    );
    assert!(rows
        .iter()
        .all(|t| t.0 != owner && t.1 != owner && t.0 < t.1));
    assert!(rows.iter().all(|t| t.0 != digest && t.1 != digest));
}
