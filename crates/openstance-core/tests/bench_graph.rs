//! P2-CORE-07: the synthetic graph and query sets behind `benches/search.rs`
//! must be what the latency gate claims to measure: people with titles,
//! companies and locations, subjects in FTS, a vector per searchable person,
//! text queries that take the fused path and structured queries that match.
//! Runs a small preset with the hash embedder (no model, no network).

#[path = "../benches/support/perf_graph.rs"]
mod perf_graph;

use perf_graph::{build, structured_queries, text_queries, PerfParams};

fn small() -> PerfParams {
    PerfParams {
        people: 1_500,
        interactions: 15_000,
        seed: 7,
        embedder: "hash:64".into(),
    }
}

#[test]
fn perf_graph_has_people_fts_and_vectors() {
    let g = build(&small());
    let c = g.conn();
    let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(n("SELECT COUNT(*) FROM person"), 1_500);
    assert_eq!(n("SELECT COUNT(*) FROM person WHERE is_self = 1"), 1);
    let bulk = n("SELECT COUNT(*) FROM person WHERE is_bulk = 1");
    assert!((20..=80).contains(&bulk), "about 3% bulk, got {bulk}");
    assert!(n("SELECT COUNT(*) FROM person WHERE company IS NOT NULL") > 1_000);
    assert_eq!(n("SELECT COUNT(*) FROM interaction"), 15_000);
    assert!(n("SELECT COUNT(*) FROM interaction WHERE kind = 'meeting'") > 0);
    let searchable = n("SELECT COUNT(*) FROM person WHERE is_bulk = 0 AND is_self = 0");
    assert_eq!(n("SELECT COUNT(*) FROM person_fts"), searchable);
    assert_eq!(n("SELECT COUNT(*) FROM person_vec"), searchable);
    assert!(g.embedding_warning().is_none());
}

#[test]
fn text_queries_take_the_fused_path() {
    let g = build(&small());
    let qs = text_queries();
    assert!(qs.len() >= 8);
    for (id, req) in &qs {
        let r = g.search(req).unwrap();
        assert!(!r.results.is_empty(), "{id}: no results");
        assert!(r.warnings.is_empty(), "{id}: {:?}", r.warnings);
        assert!(
            r.results
                .iter()
                .any(|x| x.relevance.as_ref().is_some_and(|r| r.vector.is_some())),
            "{id}: vector half did not run"
        );
    }
}

#[test]
fn structured_queries_match() {
    let g = build(&small());
    let qs = structured_queries();
    assert!(qs.len() >= 8);
    for (id, req) in &qs {
        assert!(req.text.is_none(), "{id}: structured queries have no text");
        let r = g.search(req).unwrap();
        assert!(!r.results.is_empty(), "{id}: no results");
    }
}

#[test]
fn build_is_deterministic() {
    let a = build(&small());
    let b = build(&small());
    let dump = |g: &openstance_core::Graph| {
        g.conn()
            .prepare("SELECT id, display_name, company, title, score FROM person ORDER BY id")
            .unwrap()
            .query_map([], |r| {
                Ok(format!(
                    "{}|{:?}|{:?}|{:?}|{}",
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, f64>(4)?
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(dump(&a), dump(&b));
}
