//! P2-CORE-06: the embedder tiers of `evals/search.toml`.
//!
//! The rules tier (FTS only, NullEmbedder) lives in
//! `crates/openstance-core/tests/search_eval.rs`. This file runs the same
//! eval file through hybrid FTS + vector fusion:
//!
//! - offline, with the deterministic `hash:64` embedder: fusion must not
//!   drop the untiered queries below the rules-tier thresholds, and every
//!   `tier = "embedder"` query must really be out of reach of FTS alone;
//! - with fastembed bge-small-en-v1.5, all 50 queries against the
//!   `[embedder]` thresholds. That test needs the model files: it runs when
//!   `OPENSTANCE_MODEL_DIR` points at a populated model cache and is
//!   skipped (with a message) otherwise, so CI stays offline.
//!
//! `cargo test -p openstance-fixtures --test search_eval_tiers -- --nocapture`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::search::{timezone, today_start, SearchRequest};
use openstance_core::{config, FixedClock, Graph, PersonId, Providers, QueryPatch};
use openstance_model::embed::{embedder_from_spec, EmbedderOptions};
use openstance_model::Embedder;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct EvalFile {
    settings: Settings,
    thresholds: Thresholds,
    embedder: Thresholds,
    people: BTreeMap<String, String>,
    q: Vec<EvalQuery>,
}

#[derive(Debug, Deserialize)]
struct Settings {
    parser: String,
    reranker: String,
    config: String,
    now: String,
    fixtures: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Thresholds {
    #[serde(default)]
    embedder: Option<String>,
    recall_at_5: f64,
    mrr: f64,
    must_not_violations: usize,
}

#[derive(Debug, Deserialize)]
struct EvalQuery {
    id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    filters: QueryPatch,
    expect_top5: Vec<String>,
    #[serde(default)]
    must_not: Vec<String>,
    #[serde(default)]
    tier: Option<String>,
}

impl EvalQuery {
    fn semantic(&self) -> bool {
        self.tier.as_deref() == Some("embedder")
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> EvalFile {
    let text =
        std::fs::read_to_string(repo().join("evals/search.toml")).expect("evals/search.toml");
    toml::from_str(&text).expect("valid eval file")
}

/// Per-query outcome.
struct Outcome {
    id: String,
    recall: Option<f64>,
    rr: Option<f64>,
    violations: usize,
}

struct Metrics {
    recall: f64,
    mrr: f64,
    violations: usize,
    outcomes: Vec<Outcome>,
}

fn mean(v: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = v.collect();
    if v.is_empty() {
        1.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Import the eval fixtures with `embedder` (vectors are built on import)
/// and run the queries `keep` selects.
fn run(
    eval: &EvalFile,
    spec: &str,
    embedder: Box<dyn Embedder>,
    keep: impl Fn(&EvalQuery) -> bool,
) -> Metrics {
    let root = repo();
    let cfg_text = std::fs::read_to_string(root.join(&eval.settings.config)).unwrap();
    let (mut cfg, _) = config::parse_str(&cfg_text, true).unwrap();
    cfg.model.parser = eval.settings.parser.clone();
    cfg.model.reranker = eval.settings.reranker.clone();
    cfg.model.embedder = spec.to_string();
    let now = time::OffsetDateTime::parse(
        &eval.settings.now,
        &time::format_description::well_known::Rfc3339,
    )
    .expect("settings.now is RFC 3339")
    .unix_timestamp();
    let tz = timezone(&cfg);
    let opts = openstance_import::ImportOptions {
        tz,
        ..openstance_import::ImportOptions::default()
    };
    let mut providers = Providers::with_importers(openstance_import::registry(&opts));
    providers.models.embedder = embedder;
    let mut g = Graph::open_in_memory(cfg, providers, Box::new(FixedClock(now))).unwrap();
    let files: Vec<PathBuf> = eval
        .settings
        .fixtures
        .iter()
        .map(|f| root.join(f))
        .collect();
    g.import(&files, &ImportRequest::default()).unwrap();

    let people: BTreeMap<&str, PersonId> = eval
        .people
        .iter()
        .map(|(k, h)| {
            let (kind, value) = h.split_once(':').expect("kind:value handle");
            let id = g
                .conn()
                .query_row(
                    "SELECT person_id FROM identity WHERE kind = ?1 AND value = ?2",
                    [kind, value],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("handle {h} not in the graph: {e}"));
            (k.as_str(), id)
        })
        .collect();
    let key_of: BTreeMap<PersonId, &str> = people.iter().map(|(k, id)| (*id, *k)).collect();
    let id = |k: &str| {
        *people
            .get(k)
            .unwrap_or_else(|| panic!("unknown person key {k}"))
    };

    let mut outcomes = Vec::new();
    for q in eval.q.iter().filter(|q| keep(q)) {
        let resp = g
            .search(&SearchRequest {
                text: q.text.clone(),
                filters: q.filters.clone(),
                today: today_start(now, tz),
                now,
                ..SearchRequest::default()
            })
            .unwrap();
        let ranked: Vec<PersonId> = resp.results.iter().map(|r| r.person.id).collect();
        let top5 = &ranked[..ranked.len().min(5)];
        let expected: Vec<PersonId> = q.expect_top5.iter().map(|k| id(k)).collect();
        let hit = expected.iter().filter(|e| top5.contains(e)).count();
        let recall = (!expected.is_empty()).then(|| hit as f64 / expected.len() as f64);
        let rr = (!expected.is_empty()).then(|| {
            ranked
                .iter()
                .position(|r| expected.contains(r))
                .map_or(0.0, |p| 1.0 / (p + 1) as f64)
        });
        let violations = q.must_not.iter().filter(|k| top5.contains(&id(k))).count();
        let names: Vec<&str> = top5
            .iter()
            .map(|p| key_of.get(p).copied().unwrap_or("?"))
            .collect();
        let ok = recall.is_none_or(|r| r >= 1.0) && violations == 0;
        println!(
            "  {} {:<4} {:<8} recall {:<5} rr {:<5} top5 {:?}  {}",
            if ok { "ok  " } else { "MISS" },
            q.id,
            q.tier.as_deref().unwrap_or("-"),
            recall.map_or("-".into(), |r| format!("{r:.2}")),
            rr.map_or("-".into(), |r| format!("{r:.2}")),
            names,
            q.text.as_deref().unwrap_or("(filters only)")
        );
        outcomes.push(Outcome {
            id: q.id.clone(),
            recall,
            rr,
            violations,
        });
    }
    let m = Metrics {
        recall: mean(outcomes.iter().filter_map(|o| o.recall)),
        mrr: mean(outcomes.iter().filter_map(|o| o.rr)),
        violations: outcomes.iter().map(|o| o.violations).sum(),
        outcomes,
    };
    println!(
        "{spec}: {} queries  Recall@5 {:.3}  MRR {:.3}  must_not violations {}",
        m.outcomes.len(),
        m.recall,
        m.mrr,
        m.violations
    );
    m
}

fn assert_meets(m: &Metrics, t: &Thresholds, what: &str) {
    assert!(
        m.violations <= t.must_not_violations,
        "{what}: must_not violations {} > {}",
        m.violations,
        t.must_not_violations
    );
    assert!(
        m.recall >= t.recall_at_5,
        "{what}: Recall@5 {:.3} < {:.2}",
        m.recall,
        t.recall_at_5
    );
    assert!(m.mrr >= t.mrr, "{what}: MRR {:.3} < {:.2}", m.mrr, t.mrr);
}

fn hash() -> Box<dyn Embedder> {
    embedder_from_spec("hash:64", &EmbedderOptions::default()).unwrap()
}

#[test]
fn eval_file_has_fifty_queries_and_an_embedder_tier() {
    let eval = load();
    assert!(eval.q.len() >= 50, "at least 50 queries (P2-CORE-06)");
    let semantic = eval.q.iter().filter(|q| q.semantic()).count();
    assert!(
        semantic >= 10,
        "at least 10 semantic queries, got {semantic}"
    );
    assert!(
        eval.q.iter().all(|q| q.tier.is_none() || q.semantic()),
        "only tier = \"embedder\" is known"
    );
    assert_eq!(
        eval.embedder.embedder.as_deref(),
        Some("fastembed:bge-small-en-v1.5")
    );
}

/// Hybrid fusion with a vector half that knows nothing about meaning must
/// not cost the keyword and structured queries anything.
#[test]
fn hash_embedder_fusion_keeps_untiered_queries_at_the_rules_baseline() {
    let eval = load();
    let m = run(&eval, "hash:64", hash(), |q| !q.semantic());
    assert_meets(&m, &eval.thresholds, "hash:64 fusion, untiered queries");
}

/// A query marked `tier = "embedder"` must be out of FTS reach: with FTS
/// only, its expected person is not in the top 5. Otherwise it belongs in
/// the rules tier.
#[test]
fn semantic_queries_are_out_of_reach_of_fts_alone() {
    let eval = load();
    let null = embedder_from_spec("none", &EmbedderOptions::default()).unwrap();
    let m = run(&eval, "none", null, EvalQuery::semantic);
    let answered: Vec<&str> = m
        .outcomes
        .iter()
        .filter(|o| o.recall.is_some_and(|r| r >= 1.0))
        .map(|o| o.id.as_str())
        .collect();
    assert!(
        answered.is_empty(),
        "FTS alone already answers {answered:?}; drop their tier"
    );
}

/// A populated model cache, or `None` (the test then skips).
fn model_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("OPENSTANCE_MODEL_DIR")?);
    let populated = std::fs::read_dir(&dir).ok()?.next().is_some();
    populated.then_some(dir)
}

#[test]
fn fastembed_tier_meets_the_embedder_thresholds() {
    let Some(dir) = model_dir() else {
        println!("skipped: needs model download (set OPENSTANCE_MODEL_DIR to a populated cache)");
        return;
    };
    let eval = load();
    let spec = eval.embedder.embedder.clone().unwrap();
    let opts = EmbedderOptions {
        cache_dir: dir,
        offline: true,
        ..EmbedderOptions::default()
    };
    let embedder = embedder_from_spec(&spec, &opts).unwrap();
    let m = run(&eval, &spec, embedder, |_| true);
    assert_meets(&m, &eval.embedder, &spec);
    // Turning the embedder on must not cost the untiered queries anything.
    let untiered: Vec<&Outcome> = m
        .outcomes
        .iter()
        .zip(&eval.q)
        .filter(|(_, q)| !q.semantic())
        .map(|(o, _)| o)
        .collect();
    let recall = mean(untiered.iter().filter_map(|o| o.recall));
    assert!(
        recall >= eval.thresholds.recall_at_5,
        "fastembed: untiered Recall@5 {recall:.3} < rules threshold {:.2}",
        eval.thresholds.recall_at_5
    );
}
