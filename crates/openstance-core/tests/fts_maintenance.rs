//! P1-CORE-14 acceptance on the fixture graph: FTS rows follow edits and
//! merges without a rebuild, and rebuild repopulates exactly the non-bulk,
//! non-self people.

use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::fts;
use openstance_core::{config, FixedClock, Graph, Providers};

const NOW: i64 = 1_792_497_600; // 2026-10-20T12:00:00Z

fn graph() -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("fixtures/config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let opts = openstance_import::ImportOptions::default();
    let mut g = Graph::open_in_memory(
        cfg,
        Providers::with_importers(openstance_import::registry(&opts)),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    let files: Vec<PathBuf> = [
        "fixtures/mbox/small.mbox",
        "fixtures/linkedin/Connections.csv",
        "fixtures/linkedin/messages.csv",
        "fixtures/linkedin/Invitations.csv",
        "fixtures/vcf/contacts.vcf",
    ]
    .iter()
    .map(|f| root.join(f))
    .collect();
    g.import(&files, &ImportRequest::default()).unwrap();
    g
}

fn hits(g: &Graph, text: &str) -> Vec<i64> {
    fts::candidates(g.conn(), text, None, 50)
        .unwrap()
        .into_iter()
        .map(|h| h.0)
        .collect()
}

fn count(g: &Graph, sql: &str) -> i64 {
    g.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn fts_row_count_equals_non_bulk_non_self_people_after_import_and_rebuild() {
    let mut g = graph();
    let people = "SELECT count(*) FROM person WHERE is_bulk = 0 AND is_self = 0";
    assert_eq!(
        count(&g, "SELECT count(*) FROM person_fts"),
        count(&g, people)
    );
    assert_eq!(count(&g, people), 29);
    g.rebuild().unwrap();
    assert_eq!(count(&g, "SELECT count(*) FROM person_fts"), 29);
}

#[test]
fn fts_edit_is_searchable_without_rebuild() {
    let mut g = graph();
    assert!(hits(&g, "zymurgist").is_empty());
    let id = g
        .edit(
            &PersonKey::parse("priya@northwind.example"),
            &Edit {
                set: vec![(OverrideField::Title, "Chief Zymurgist".into())],
                add_tags: vec!["brewing-circle".into()],
                ..Edit::default()
            },
        )
        .unwrap();
    assert_eq!(hits(&g, "zymurgist"), [id]);
    assert_eq!(hits(&g, "brewing-circle"), [id]);
}

#[test]
fn fts_merged_away_rowid_returns_no_hits() {
    let mut g = graph();
    let greta = g
        .who(&PersonKey::parse("in/greta-solberg"))
        .unwrap()
        .person
        .id;
    let hugo = g
        .who(&PersonKey::parse("in/hugo-brandt"))
        .unwrap()
        .person
        .id;
    assert_eq!(hits(&g, "hugo"), [hugo]);
    let out = g
        .merge(&[PersonKey::Id(greta), PersonKey::Id(hugo)])
        .unwrap();
    let gone = if out.person == greta { hugo } else { greta };
    let rows = count(
        &g,
        &format!("SELECT count(*) FROM person_fts WHERE rowid = {gone}"),
    );
    assert_eq!(rows, 0, "absorbed person {gone} still indexed");
    assert_eq!(hits(&g, "hugo"), [out.person]);
    assert_eq!(hits(&g, "greta"), [out.person]);
}

#[test]
fn fts_bulk_sender_is_never_indexed() {
    let g = graph();
    assert!(hits(&g, "digest").is_empty());
    assert!(hits(&g, "news@digest.example").is_empty());
}
