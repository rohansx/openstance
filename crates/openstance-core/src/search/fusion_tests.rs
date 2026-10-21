//! P2-CORE-03: hybrid FTS + vector fusion through `search()`, with the
//! deterministic HashEmbedder (no model download).

use openstance_model::embed::{HashEmbedder, UnavailableEmbedder};
use openstance_model::{EmbedKind, Embedder, Models, NullEmbedder, QueryPatch};
use rusqlite::Connection;

use super::testkit::{indexed, NOW};
use super::*;
use crate::embed::{index, IndexOptions};
use crate::json::SearchMode;

fn req(text: &str) -> SearchRequest {
    SearchRequest {
        text: Some(text.to_string()),
        today: NOW - 12 * 3600,
        now: NOW,
        ..SearchRequest::default()
    }
}

fn models(e: Box<dyn Embedder>) -> Models {
    Models {
        embedder: e,
        ..Models::null()
    }
}

/// The seeded graph with FTS and hash vectors built.
fn vectors(e: &dyn Embedder) -> Connection {
    let mut c = indexed();
    index(&mut c, e, &IndexOptions::default(), &mut |_| {}).unwrap();
    c
}

fn ids(r: &SearchResponse) -> Vec<i64> {
    r.results.iter().map(|x| x.person.id).collect()
}

#[test]
fn text_search_fuses_both_halves_and_exposes_raw_signals() {
    let e = HashEmbedder::new(384);
    let c = vectors(&e);
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(e)),
        &req("devtools dinner"),
    )
    .unwrap();
    assert_eq!(r.mode, SearchMode::Text);
    assert_eq!(r.models.embedder, "hash:384");
    assert_eq!(ids(&r)[0], 7, "{:?}", ids(&r));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let top = r.results[0].relevance.unwrap();
    assert!(top.fts_rank.is_some() && top.vec_sim.is_some(), "{top:?}");
    assert!(top.vector.is_some());
    let v = serde_json::to_value(&r.results[0]).unwrap();
    for k in ["fts", "vector", "fused", "final", "fts_rank", "vec_sim"] {
        assert!(v["relevance"].get(k).is_some(), "missing {k}: {v}");
    }
    // every candidate came from at least one half
    assert!(
        r.results
            .iter()
            .all(|x| x.relevance.unwrap().fts_rank.is_some()
                || x.relevance.unwrap().vec_sim.is_some())
    );
}

#[test]
fn a_semantic_only_hit_is_found_and_labelled() {
    // "dinnerparty" never matches FTS ("dinnerparty*"), but shares the
    // 5-character prefix "dinne" with "dinner" in the hash embedder.
    let e = HashEmbedder::new(384);
    let c = vectors(&e);
    let fts_only = search(&c, &Config::default(), &Models::null(), &req("dinnerparty")).unwrap();
    assert!(fts_only.results.is_empty());
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(e)),
        &req("dinnerparty"),
    )
    .unwrap();
    assert!(!r.results.is_empty());
    let top = &r.results[0];
    assert!([3, 7].contains(&top.person.id), "{:?}", ids(&r));
    let rel = top.relevance.unwrap();
    assert_eq!(rel.fts, 0.0);
    assert_eq!(rel.fts_rank, None);
    assert!(rel.vec_sim.unwrap() > 0.0);
    let sem = top
        .matched
        .iter()
        .find(|m| m.field == "semantic")
        .expect("semantic match entry");
    assert_eq!(sem.similarity, Some(round3(rel.vec_sim.unwrap())));
}

#[test]
fn structured_filters_restrict_the_vector_half() {
    let e = HashEmbedder::new(384);
    let c = vectors(&e);
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(e)),
        &SearchRequest {
            filters: QueryPatch {
                company: Some(vec!["northwind".into()]),
                ..QueryPatch::default()
            },
            ..req("dinnerparty")
        },
    )
    .unwrap();
    assert!(ids(&r).iter().all(|id| *id == 2), "{:?}", ids(&r));
}

#[test]
fn without_an_embedder_search_is_fts_only_as_in_phase_1() {
    let c = vectors(&HashEmbedder::new(384));
    let with_null = search(
        &c,
        &Config::default(),
        &models(Box::new(NullEmbedder)),
        &req("devtools"),
    )
    .unwrap();
    assert_eq!(ids(&with_null), [7, 3]);
    let rel = with_null.results[0].relevance.unwrap();
    assert_eq!((rel.vector, rel.vec_sim), (None, None));
    assert!(rel.fts_rank.is_some());
    assert!(with_null.warnings.is_empty());
}

#[test]
fn a_changed_embedder_skips_the_vector_half_with_a_warning() {
    let c = vectors(&HashEmbedder::new(384));
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(HashEmbedder::new(64))),
        &req("devtools"),
    )
    .unwrap();
    assert_eq!(ids(&r), [7, 3]);
    assert!(r.results[0].relevance.unwrap().vector.is_none());
    assert_eq!(
        r.warnings,
        ["embeddings out of date: run 'openstance rebuild --embeddings'"]
    );
}

#[test]
fn an_unavailable_model_falls_back_to_fts_with_a_warning() {
    struct Same(HashEmbedder);
    impl Embedder for Same {
        fn id(&self) -> &str {
            "fastembed:bge-small-en-v1.5"
        }
        fn dims(&self) -> usize {
            384
        }
        fn is_remote(&self) -> bool {
            false
        }
        fn embed(&self, t: &[&str], k: EmbedKind) -> openstance_model::Result<Vec<Vec<f32>>> {
            self.0.embed(t, k)
        }
    }
    let c = vectors(&Same(HashEmbedder::new(384)));
    let gone = UnavailableEmbedder {
        id: "fastembed:bge-small-en-v1.5".into(),
        dims: 384,
        reason: "model not downloaded".into(),
    };
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(gone)),
        &req("devtools"),
    )
    .unwrap();
    assert_eq!(ids(&r), [7, 3]);
    assert_eq!(r.warnings.len(), 1);
    assert!(
        r.warnings[0].starts_with("embedder unavailable:")
            && r.warnings[0].contains("rebuild --embeddings"),
        "{:?}",
        r.warnings
    );
}

#[test]
fn structured_mode_never_embeds_the_query() {
    struct Panics;
    impl Embedder for Panics {
        fn id(&self) -> &str {
            "hash:384"
        }
        fn dims(&self) -> usize {
            384
        }
        fn is_remote(&self) -> bool {
            false
        }
        fn embed(&self, _: &[&str], _: EmbedKind) -> openstance_model::Result<Vec<Vec<f32>>> {
            panic!("structured queries must not embed")
        }
    }
    let c = vectors(&HashEmbedder::new(384));
    let r = search(
        &c,
        &Config::default(),
        &models(Box::new(Panics)),
        &SearchRequest {
            text: None,
            filters: QueryPatch {
                company: Some(vec!["contoso".into()]),
                ..QueryPatch::default()
            },
            ..req("")
        },
    )
    .unwrap();
    assert_eq!(r.mode, SearchMode::Structured);
}
