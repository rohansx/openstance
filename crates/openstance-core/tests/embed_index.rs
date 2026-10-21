//! P2-CORE-02 acceptance on the fixture graph through the Graph facade:
//! import embeds every non-bulk, non-self person; editing one title
//! re-embeds exactly one person; rebuild with no source change re-embeds no
//! one; merges drop the absorbed vector; a changed embedder is detected.
//! Uses the deterministic HashEmbedder (no model, no network).

use std::path::{Path, PathBuf};

use openstance_core::embed::{IndexOptions, Status, CHANGED_WARNING};
use openstance_core::ingest::ImportRequest;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::{config, FixedClock, Graph, Providers};
use openstance_model::embed::HashEmbedder;
use openstance_model::Embedder;

const NOW: i64 = 1_792_497_600; // 2026-10-20T12:00:00Z

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn providers(e: Box<dyn Embedder>) -> Providers {
    let opts = openstance_import::ImportOptions::default();
    let mut p = Providers::with_importers(openstance_import::registry(&opts));
    p.models.embedder = e;
    p
}

fn cfg() -> openstance_core::Config {
    let text = std::fs::read_to_string(root().join("fixtures/config.toml")).unwrap();
    config::parse_str(&text, true).unwrap().0
}

fn files() -> Vec<PathBuf> {
    [
        "fixtures/mbox/small.mbox",
        "fixtures/linkedin/Connections.csv",
        "fixtures/linkedin/messages.csv",
        "fixtures/linkedin/Invitations.csv",
        "fixtures/vcf/contacts.vcf",
    ]
    .iter()
    .map(|f| root().join(f))
    .collect()
}

fn count(g: &Graph, sql: &str) -> i64 {
    g.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

const ELIGIBLE: &str = "SELECT count(*) FROM person WHERE is_bulk = 0 AND is_self = 0";

#[test]
fn import_edit_rebuild_and_merge_keep_vectors_current_incrementally() {
    let mut g = Graph::open_in_memory(
        cfg(),
        providers(Box::new(HashEmbedder::new(384))),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    g.on_embed_progress(move |p| s2.lock().unwrap().push((p.done, p.total)));
    g.import(&files(), &ImportRequest::default()).unwrap();
    let people = count(&g, ELIGIBLE);
    assert!(people > 10);
    let r = g.last_embed_report().unwrap().clone();
    assert_eq!(r.embedded as i64, people);
    assert_eq!(count(&g, "SELECT count(*) FROM person_vec"), people);
    assert_eq!(g.embedding_status().unwrap(), Status::Ready);
    assert_eq!(
        seen.lock().unwrap().last().copied(),
        Some((people as usize, people as usize))
    );

    // edit one title -> exactly one re-embed
    let id: i64 = g
        .conn()
        .query_row(
            "SELECT id FROM person WHERE is_bulk = 0 AND is_self = 0 ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let edit = Edit {
        set: vec![(OverrideField::Title, "Chief Pastry Officer".into())],
        ..Edit::default()
    };
    g.edit(&PersonKey::Id(id), &edit).unwrap();
    let r = g.last_embed_report().unwrap();
    assert_eq!(r.embedded, 1, "{r:?}");

    // rebuild with no source change -> no one re-embedded
    g.rebuild().unwrap();
    let r = g.last_embed_report().unwrap();
    assert_eq!(r.embedded, 0, "{r:?}");
    assert_eq!(r.unchanged as i64, people);

    // a merge removes the absorbed person's vector
    let two: Vec<i64> = g
        .conn()
        .prepare("SELECT id FROM person WHERE is_bulk = 0 AND is_self = 0 ORDER BY id LIMIT 2")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    g.merge(&[PersonKey::Id(two[0]), PersonKey::Id(two[1])])
        .unwrap();
    assert_eq!(
        count(&g, "SELECT count(*) FROM person_vec"),
        count(&g, ELIGIBLE)
    );
    assert_eq!(
        count(&g, "SELECT count(*) FROM person_embed"),
        count(&g, ELIGIBLE)
    );
}

#[test]
fn a_changed_embedder_warns_until_rebuild_embeddings() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("g.db");
    {
        let mut g = Graph::open(
            cfg(),
            &db,
            providers(Box::new(HashEmbedder::new(384))),
            Box::new(FixedClock(NOW)),
        )
        .unwrap();
        g.import(&files(), &ImportRequest::default()).unwrap();
        assert_eq!(g.embedding_warning(), None);
    }
    let mut g = Graph::open(
        cfg(),
        &db,
        providers(Box::new(HashEmbedder::new(32))),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    assert_eq!(g.embedding_warning().as_deref(), Some(CHANGED_WARNING));
    // a plain rebuild does not touch the vectors
    g.rebuild().unwrap();
    assert_eq!(
        g.last_embed_report().unwrap().skipped.as_deref(),
        Some(CHANGED_WARNING)
    );
    assert!(matches!(
        g.embedding_status().unwrap(),
        Status::Changed { .. }
    ));
    let r = g
        .embed(&IndexOptions {
            force: true,
            ..IndexOptions::default()
        })
        .unwrap();
    assert!(r.recreated);
    assert_eq!(r.embedded as i64, count(&g, ELIGIBLE));
    assert_eq!(g.embedding_warning(), None);
    assert_eq!(g.embedding_status().unwrap(), Status::Ready);
}
