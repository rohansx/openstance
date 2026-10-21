//! P1-CORE-17: first-tier search evaluation over the fixture graph.
//! Reads `evals/search.toml`, imports the fixtures, runs every query with
//! the rules parser and no reranker, prints Recall@5 / MRR / must_not
//! violations (with per-query and per-tag detail) and fails below the
//! thresholds recorded in the file.
//!
//! `cargo test -p openstance-core --test search_eval -- --nocapture`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::search::{timezone, today_start, SearchRequest};
use openstance_core::{config, FixedClock, Graph, PersonId, Providers, QueryPatch};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalFile {
    settings: Settings,
    thresholds: Thresholds,
    /// Embedder-tier thresholds (P2-CORE-06); the rules tier ignores them.
    #[serde(default)]
    #[allow(dead_code)]
    embedder: Option<EmbedderTier>,
    people: BTreeMap<String, String>,
    q: Vec<EvalQuery>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    parser: String,
    reranker: String,
    config: String,
    now: String,
    fixtures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Thresholds {
    recall_at_5: f64,
    mrr: f64,
    must_not_violations: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct EmbedderTier {
    embedder: String,
    recall_at_5: f64,
    mrr: f64,
    must_not_violations: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    tags: Vec<String>,
    /// `embedder`: semantic phrasing, run only by the embedder tier.
    #[serde(default)]
    tier: Option<String>,
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_now(s: &str) -> i64 {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .expect("settings.now is RFC 3339")
        .unix_timestamp()
}

fn person_of(g: &Graph, handle: &str) -> PersonId {
    let (kind, value) = handle.split_once(':').expect("kind:value handle");
    g.conn()
        .query_row(
            "SELECT person_id FROM identity WHERE kind = ?1 AND value = ?2",
            [kind, value],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("handle {handle} not in the graph: {e}"))
}

#[derive(Default)]
struct Tally {
    recall: Vec<f64>,
    rr: Vec<f64>,
    violations: usize,
}

impl Tally {
    fn mean(v: &[f64]) -> f64 {
        if v.is_empty() {
            1.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    }
}

#[test]
fn search_eval_rules_tier() {
    let root = repo();
    let text = std::fs::read_to_string(root.join("evals/search.toml")).expect("evals/search.toml");
    let eval: EvalFile = toml::from_str(&text).expect("valid eval file");
    assert_eq!(
        eval.settings.parser, "rules",
        "Phase 1 tier runs the rules parser"
    );
    assert_eq!(eval.settings.reranker, "none");
    assert!(eval.q.len() >= 50, "at least 50 queries (P2-CORE-06)");
    for q in &eval.q {
        assert!(
            matches!(q.tier.as_deref(), None | Some("embedder")),
            "{}: unknown tier {:?}",
            q.id,
            q.tier
        );
    }

    let cfg_text = std::fs::read_to_string(root.join(&eval.settings.config)).unwrap();
    let (mut cfg, _) = config::parse_str(&cfg_text, true).unwrap();
    cfg.model.parser = eval.settings.parser.clone();
    cfg.model.reranker = eval.settings.reranker.clone();
    let now = parse_now(&eval.settings.now);
    let tz = timezone(&cfg);
    let opts = openstance_import::ImportOptions {
        tz,
        ..openstance_import::ImportOptions::default()
    };
    let mut g = Graph::open_in_memory(
        cfg,
        Providers::with_importers(openstance_import::registry(&opts)),
        Box::new(FixedClock(now)),
    )
    .unwrap();
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
        .map(|(k, h)| (k.as_str(), person_of(&g, h)))
        .collect();
    let key_of: BTreeMap<PersonId, &str> = people.iter().map(|(k, id)| (*id, *k)).collect();
    let id = |k: &str| {
        *people
            .get(k)
            .unwrap_or_else(|| panic!("unknown person key {k}"))
    };

    let mut all = Tally::default();
    let mut by_tag: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures = Vec::new();
    let skipped: Vec<&str> = eval
        .q
        .iter()
        .filter(|q| q.tier.is_some())
        .map(|q| q.id.as_str())
        .collect();
    println!(
        "search eval (rules tier): {} queries, {} embedder-tier skipped",
        eval.q.len() - skipped.len(),
        skipped.len()
    );
    for q in eval.q.iter().filter(|q| q.tier.is_none()) {
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
        let violations: Vec<&str> = q
            .must_not
            .iter()
            .filter(|k| top5.contains(&id(k)))
            .map(String::as_str)
            .collect();
        let names: Vec<&str> = top5
            .iter()
            .map(|p| key_of.get(p).copied().unwrap_or("?"))
            .collect();
        let ok = recall.is_none_or(|r| r >= 1.0) && violations.is_empty();
        println!(
            "  {} {:<4} recall {:<5} rr {:<5} top5 {:?}  {}",
            if ok { "ok  " } else { "MISS" },
            q.id,
            recall.map_or("-".into(), |r| format!("{r:.2}")),
            rr.map_or("-".into(), |r| format!("{r:.2}")),
            names,
            q.text.as_deref().unwrap_or("(filters only)")
        );
        if !ok {
            failures.push(q.id.clone());
        }
        let record = |t: &mut Tally| {
            if let Some(r) = recall {
                t.recall.push(r);
            }
            if let Some(r) = rr {
                t.rr.push(r);
            }
            t.violations += violations.len();
        };
        record(&mut all);
        for tag in &q.tags {
            record(by_tag.entry(tag.clone()).or_default());
        }
    }
    let recall = Tally::mean(&all.recall);
    let mrr = Tally::mean(&all.rr);
    println!(
        "Recall@5 {recall:.3} (min {:.2})  MRR {mrr:.3} (min {:.2})  must_not violations {} (max {})",
        eval.thresholds.recall_at_5,
        eval.thresholds.mrr,
        all.violations,
        eval.thresholds.must_not_violations
    );
    for (tag, t) in &by_tag {
        println!(
            "  tag {tag:<14} recall {:.3}  mrr {:.3}  violations {}",
            Tally::mean(&t.recall),
            Tally::mean(&t.rr),
            t.violations
        );
    }
    if !skipped.is_empty() {
        println!("embedder tier only (skipped here): {}", skipped.join(", "));
    }
    if !failures.is_empty() {
        println!("queries below perfect: {}", failures.join(", "));
    }
    assert!(
        all.violations <= eval.thresholds.must_not_violations,
        "must_not violations {}",
        all.violations
    );
    assert!(
        recall >= eval.thresholds.recall_at_5,
        "Recall@5 {recall:.3} below {}",
        eval.thresholds.recall_at_5
    );
    assert!(
        mrr >= eval.thresholds.mrr,
        "MRR {mrr:.3} below {}",
        eval.thresholds.mrr
    );
}
