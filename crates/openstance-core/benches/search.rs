//! P2-CORE-07 latency gate: criterion benches for text search with the
//! embedder (`search/text_fused/*`, query embedding included) and
//! structured search (`search/structured/*`) on the perf fixture
//! (20k people, 500k interactions; see `support/perf_graph.rs`).
//!
//! ```sh
//! cargo bench -p openstance-core --bench search
//! # real model (needs the bge-small model in OPENSTANCE_MODEL_DIR):
//! OPENSTANCE_BENCH_EMBEDDER=fastembed:bge-small-en-v1.5 \
//!   cargo bench -p openstance-core --bench search --features openstance-model/fastembed
//! ```
//!
//! After criterion, a gate measures p50/p95 per group over the whole query
//! set and fails the run (exit 1) when
//! - text search p50 is over 300 ms or structured search p50 over 50 ms
//!   (performance-targets.md), or
//! - `OPENSTANCE_BENCH_COMPARE=<file>` names a previous run's
//!   `OPENSTANCE_BENCH_SAVE=<file>` output and a group's p50 is more than
//!   20 % slower than it (the CI regression budget for this gate).
//!
//! Sizes: `OPENSTANCE_BENCH_PEOPLE`, `OPENSTANCE_BENCH_INTERACTIONS`.
//! `OPENSTANCE_BENCH_GATE=0` skips the gate. `OPENSTANCE_BENCH_EXPLAIN=1`
//! prints each query's per-stage timings (`--explain`) once, then exits.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion};
use openstance_core::search::SearchRequest;
use openstance_core::Graph;

mod support;
use support::perf_graph::{build, structured_queries, text_queries, PerfParams};

/// p50 targets in ms (performance-targets.md, P2-CORE-07).
const TEXT_P50_MS: f64 = 300.0;
const STRUCTURED_P50_MS: f64 = 50.0;
/// Allowed p50 slowdown against a saved run.
const MAX_REGRESSION: f64 = 0.20;
/// Timed runs per query in the gate.
const GATE_RUNS: usize = 15;

fn bench_group(c: &mut Criterion, g: &Graph, name: &str, qs: &[(String, SearchRequest)]) {
    let mut grp = c.benchmark_group(name);
    grp.sample_size(20)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    for (id, req) in qs {
        grp.bench_with_input(BenchmarkId::from_parameter(id), req, |b, req| {
            b.iter(|| g.search(black_box(req)).expect("search"))
        });
    }
    grp.finish();
}

/// Sorted per-call latencies in ms over every query of the set.
fn samples(g: &Graph, qs: &[(String, SearchRequest)]) -> Vec<f64> {
    let mut out = Vec::with_capacity(qs.len() * GATE_RUNS);
    for (_, req) in qs {
        g.search(req).expect("warm-up search");
        for _ in 0..GATE_RUNS {
            let t = Instant::now();
            black_box(g.search(black_box(req)).expect("search"));
            out.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

/// Check targets and the regression budget; `Err` lists every failure.
fn gate(p50: &BTreeMap<String, f64>) -> Result<(), Vec<String>> {
    let mut fails = Vec::new();
    for (group, target) in [
        ("search/text_fused", TEXT_P50_MS),
        ("search/structured", STRUCTURED_P50_MS),
    ] {
        if let Some(v) = p50.get(group) {
            if *v > target {
                fails.push(format!(
                    "{group}: p50 {v:.1} ms over the {target} ms target"
                ));
            }
        }
    }
    if let Some(path) = std::env::var_os("OPENSTANCE_BENCH_COMPARE") {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("OPENSTANCE_BENCH_COMPARE {}: {e}", path.to_string_lossy()));
        let base: BTreeMap<String, f64> = serde_json::from_str(&text).expect("baseline json");
        for (group, now) in p50 {
            if let Some(was) = base.get(group) {
                let change = now / was - 1.0;
                eprintln!(
                    "{group}: p50 {was:.2} -> {now:.2} ms ({:+.1}%)",
                    change * 100.0
                );
                if change > MAX_REGRESSION {
                    fails.push(format!(
                        "{group}: p50 {now:.2} ms is {:.0}% slower than baseline {was:.2} ms (budget {:.0}%)",
                        change * 100.0,
                        MAX_REGRESSION * 100.0
                    ));
                }
            }
        }
    }
    if fails.is_empty() {
        Ok(())
    } else {
        Err(fails)
    }
}

fn main() -> ExitCode {
    let p = PerfParams::from_env();
    let t = Instant::now();
    let g = build(&p);
    eprintln!(
        "perf graph: {} people, {} interactions, embedder {} (built in {:.1} s)",
        p.people,
        p.interactions,
        p.embedder,
        t.elapsed().as_secs_f64()
    );
    let sets = [
        ("search/text_fused", text_queries()),
        ("search/structured", structured_queries()),
    ];

    if std::env::var("OPENSTANCE_BENCH_EXPLAIN").as_deref() == Ok("1") {
        for (name, qs) in &sets {
            for (id, req) in qs {
                let mut req = req.clone();
                req.explain = true;
                let r = g.search(&req).expect("search");
                eprintln!("{name}/{id}: {} results, {:?}", r.total, r.timings_ms);
            }
        }
        return ExitCode::SUCCESS;
    }

    let mut c = Criterion::default().configure_from_args();
    for (name, qs) in &sets {
        bench_group(&mut c, &g, name, qs);
    }
    c.final_summary();

    // `cargo test --benches` / `cargo bench -- --test` only smoke-run.
    let smoke = std::env::args().any(|a| a == "--test");
    if smoke || std::env::var("OPENSTANCE_BENCH_GATE").as_deref() == Ok("0") {
        return ExitCode::SUCCESS;
    }
    let mut p50 = BTreeMap::new();
    for (name, qs) in &sets {
        let s = samples(&g, qs);
        eprintln!(
            "gate {name}: p50 {:.2} ms, p95 {:.2} ms over {} calls",
            pct(&s, 0.5),
            pct(&s, 0.95),
            s.len()
        );
        p50.insert((*name).to_string(), pct(&s, 0.5));
    }
    if let Some(path) = std::env::var_os("OPENSTANCE_BENCH_SAVE") {
        let json = serde_json::to_string_pretty(&p50).expect("json");
        std::fs::write(&path, json).expect("OPENSTANCE_BENCH_SAVE");
    }
    match gate(&p50) {
        Ok(()) => ExitCode::SUCCESS,
        Err(fails) => {
            for f in fails {
                eprintln!("latency gate: {f}");
            }
            ExitCode::FAILURE
        }
    }
}
