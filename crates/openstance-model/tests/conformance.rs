//! Embedder conformance (models.md section 11) for every backend that is
//! available offline, plus the config factory (P2-MODEL-02).
//!
//! The fastembed tier needs the model: it runs only when
//! `OPENSTANCE_MODEL_DIR` points at a populated cache (or with
//! `cargo test -- --ignored` on a machine with network).

use openstance_model::embed::{
    embedder_from_spec, l2_normalize, query_prefix, EmbedderOptions, HashEmbedder,
};
use openstance_model::{EmbedKind, Embedder, ModelError};

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn conformance(e: &dyn Embedder) {
    let dims = e.dims();
    assert!(dims > 0);
    // empty input
    assert!(e.embed(&[], EmbedKind::Passage).unwrap().is_empty());
    // unicode + empty string
    let v = e
        .embed(&["Zoë Ångström 東京 🚀", ""], EmbedKind::Passage)
        .unwrap();
    assert_eq!(v.len(), 2);
    for x in &v {
        assert_eq!(x.len(), dims);
        let n: f32 = x.iter().map(|f| f * f).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-3, "L2-normalized, got {n}");
        assert!(x.iter().all(|f| f.is_finite()));
    }
    // determinism
    let a = e
        .embed(&["developer tools founder"], EmbedKind::Query)
        .unwrap();
    let b = e
        .embed(&["developer tools founder"], EmbedKind::Query)
        .unwrap();
    assert_eq!(a, b);
    // batch of 1,000
    let texts: Vec<String> = (0..1000).map(|i| format!("person {i} engineer")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let out = e.embed(&refs, EmbedKind::Passage).unwrap();
    assert_eq!(out.len(), 1000);
    assert!(out.iter().all(|v| v.len() == dims));
}

#[test]
fn hash_embedder_conforms() {
    conformance(&HashEmbedder::new(384));
    assert_eq!(HashEmbedder::new(384).id(), "hash:384");
    assert!(!HashEmbedder::new(384).is_remote());
}

#[test]
fn hash_embedder_puts_shared_words_closer() {
    let e = HashEmbedder::new(384);
    let v = e
        .embed(
            &[
                "developer tools platform",
                "devtools SDK developer tools",
                "pastry chef bakery",
            ],
            EmbedKind::Passage,
        )
        .unwrap();
    assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
}

#[test]
fn l2_normalize_handles_zero_vectors() {
    let mut v = vec![3.0, 4.0];
    l2_normalize(&mut v);
    assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    let mut z = vec![0.0; 3];
    l2_normalize(&mut z);
    assert_eq!(z, vec![0.0; 3]);
}

#[test]
fn query_prefixes_follow_the_model_family() {
    assert_eq!(
        query_prefix("fastembed:bge-small-en-v1.5", EmbedKind::Query),
        "Represent this sentence for searching relevant passages: "
    );
    assert_eq!(
        query_prefix("fastembed:bge-small-en-v1.5", EmbedKind::Passage),
        ""
    );
    assert_eq!(
        query_prefix("ollama:nomic-embed-text", EmbedKind::Query),
        "search_query: "
    );
    assert_eq!(
        query_prefix("ollama:nomic-embed-text", EmbedKind::Passage),
        "search_document: "
    );
    assert_eq!(
        query_prefix("openai:text-embedding-3-small", EmbedKind::Query),
        ""
    );
    assert_eq!(query_prefix("something:else", EmbedKind::Query), "");
}

fn opts() -> EmbedderOptions {
    EmbedderOptions {
        cache_dir: std::env::temp_dir().join("openstance-model-test-nonexistent"),
        offline: true,
        ..EmbedderOptions::default()
    }
}

#[test]
fn factory_maps_none_to_the_null_embedder() {
    for spec in ["none", "", "  NONE "] {
        let e = embedder_from_spec(spec, &opts()).unwrap();
        assert_eq!(e.id(), "none");
        assert_eq!(e.dims(), 0);
    }
}

#[test]
fn factory_knows_the_hash_test_embedder() {
    let e = embedder_from_spec("hash:64", &opts()).unwrap();
    assert_eq!((e.id(), e.dims()), ("hash:64", 64));
    assert!(embedder_from_spec("hash:abc", &opts()).is_err());
}

#[test]
fn factory_keeps_the_id_of_backends_it_cannot_run() {
    // Phase 4 backends are not compiled yet: the id survives (so re-index
    // detection works) and every call says why it is unavailable.
    let e = embedder_from_spec("ollama:nomic-embed-text", &opts()).unwrap();
    assert_eq!(e.id(), "ollama:nomic-embed-text");
    assert!(matches!(
        e.embed(&["x"], EmbedKind::Query),
        Err(ModelError::Unavailable(_))
    ));
}

#[test]
fn factory_rejects_unknown_backends_and_models() {
    assert!(embedder_from_spec("bogus:thing", &opts()).is_err());
    assert!(embedder_from_spec("fastembed:no-such-model", &opts()).is_err());
    assert!(embedder_from_spec("anthropic:claude-haiku-4-5", &opts()).is_err());
}

#[test]
fn fastembed_spec_is_lazy_and_reports_384_dims_without_loading() {
    let e = embedder_from_spec("fastembed:bge-small-en-v1.5", &opts()).unwrap();
    assert_eq!(e.id(), "fastembed:bge-small-en-v1.5");
    assert_eq!(e.dims(), 384);
    assert!(!e.is_remote());
}

#[test]
fn fastembed_offline_without_a_cached_model_is_unavailable_with_a_hint() {
    let e = embedder_from_spec("fastembed:bge-small-en-v1.5", &opts()).unwrap();
    match e.embed(&["x"], EmbedKind::Query) {
        Err(ModelError::Unavailable(msg)) => {
            assert!(
                msg.contains("run once with network or set `model.embedder = \"none\"`"),
                "{msg}"
            );
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
}

/// The real model: needs a populated cache (`OPENSTANCE_MODEL_DIR`) or network.
#[cfg(feature = "fastembed")]
#[test]
#[ignore = "needs model download"]
fn fastembed_conforms_with_the_real_model() {
    let dir = std::env::var_os("OPENSTANCE_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("openstance-models"));
    let e = embedder_from_spec(
        "fastembed:bge-small-en-v1.5",
        &EmbedderOptions {
            cache_dir: dir,
            offline: false,
            ..EmbedderOptions::default()
        },
    )
    .unwrap();
    conformance(e.as_ref());
    let v = e
        .embed(
            &["developer tools", "SDK and API platform", "pastry chef"],
            EmbedKind::Passage,
        )
        .unwrap();
    assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
}
