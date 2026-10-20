# Performance targets

The numbers openstance must hit, how each one is measured, and how CI stops regressions. The targets come from the tech spec's sizing of a heavy user; the benchmark design is ours.

Related: [testing-strategy.md](testing-strategy.md), [ci-and-release.md](ci-and-release.md), [../tech-spec/data-model.md](../tech-spec/data-model.md), [../tech-spec/search.md](../tech-spec/search.md), [../tech-spec/ingestion.md](../tech-spec/ingestion.md), [../tech-spec/models.md](../tech-spec/models.md), [../architecture/data-flow.md](../architecture/data-flow.md).

## Reference workload

A heavy user's graph, from the tech spec's sizing:

| Entity | Count |
|---|---|
| people | 20,000 |
| identities | 50,000 |
| interactions (email headers, meetings, DMs) | 500,000 |
| ties | 100,000 |
| embedding vectors (384-dim f32) | 20,000 (~30 MB) |
| database file | 200-500 MB |

Reference machine: Apple M2 laptop (8-core, 16 GB RAM), SSD, on AC power, release build, nothing else running. Secondary references we also track: a 4-core x86_64 Linux machine and a Windows x86_64 laptop; their numbers are reported but only M2 numbers are gated in docs and launch claims.

## Targets

| Operation | Target (M2, reference workload) | Benchmark id | Method |
|---|---|---|---|
| Import 5 GB mbox | under 2 min, streaming, under 500 MB peak RSS | `import/mbox_5g` | xtask end-to-end, wall clock + peak RSS |
| Import LinkedIn CSV (5k rows) | under 2 s | `import/linkedin_5k` | criterion |
| Rebuild (resolution + scoring + index) | under 60 s | `rebuild/full` | xtask end-to-end, wall clock |
| First-run embedding of 20k people | under 90 s on CPU | `embed/first_run_20k` | xtask end-to-end, model pre-cached (download excluded) |
| Structured search | under 50 ms | `search/structured/*` | criterion, p50 and p95 |
| Text search with embedder | under 300 ms end to end | `search/text_fused/*` | criterion, p50 and p95, includes query embedding |
| `who`, `how_connected` | under 20 ms | `who/*`, `how_connected/*` | criterion, p95 |
| Binary size | under 40 MB without models | `size/binary` | release artifact size per target |

Interpretation:

- Latency targets are **p95** over the benchmark's query set unless stated, measured in-process (library call), plus a separate CLI wall-clock check with `hyperfine` that must stay within target + 30 ms of process start overhead.
- "Under 2 min" import and "under 60 s" rebuild are single-run wall clock, median of 3 runs.
- Model download time is excluded everywhere; the 33 MB fastembed model is pre-cached before timing.

Derived component budgets (used to locate a regression, not gated individually):

| Component | Budget |
|---|---|
| FTS5 bm25 top-200 over `person_fts` | < 10 ms |
| sqlite-vec brute-force cosine top-200 over 20k x 384 | < 10 ms |
| Query embedding, single short query (bge-small, CPU) | < 30 ms warm |
| Fusion + structured filter over <= 400 candidates | < 5 ms |
| Evidence fetch (3 most recent interactions x limit results) | < 10 ms for limit 30 |
| Scoring all 20k people (inside rebuild) | < 5 s |
| Tie derivation from 500k interactions (inside rebuild) | < 20 s |

Cold start: the first embedder call loads the ONNX model (~1 s). `serve` and `mcp` warm it at startup; the CLI pays it once per process, so the CLI wall-clock check for text search uses a warm `openstance serve` path and separately reports cold CLI time (informational).

## Synthetic data

All benchmarks run on generated data; no real exports are ever used (see [conventions.md](conventions.md#no-real-contact-data)).

```sh
cargo xtask gen-graph \
  --people 20000 --identities 50000 --interactions 500000 --ties 100000 \
  --mbox-bytes 5G --linkedin-rows 5000 \
  --seed 42 --out target/synth/heavy
```

Outputs under `target/synth/heavy/`:

| File | Used by |
|---|---|
| `inbox.mbox` (~5 GB, headers + small synthetic bodies so byte size is realistic) | `import/mbox_5g` |
| `Connections.csv` (5k rows), `messages.csv` | `import/linkedin_5k` |
| `contacts.vcf`, `calendar.ics` | rebuild input |
| `graph.db` (fully built, embeddings computed with the hash embedder) | search, who, how_connected benches |
| `graph-fastembed.db` (built with real embeddings, nightly only) | `search/text_fused` with real model |
| `queries.toml` (200 benchmark queries: 100 structured, 100 free text, sampled from the generated vocabulary) | search benches |

Generator properties:

- Deterministic per seed; the generator's version is part of the cache key.
- Zipf-distributed interactions per person (s ~ 1.1), 3% bulk senders, ~30% of people present in two or more sources, 6 years of timestamps with recency skew, meeting titles and subjects drawn from a synthetic vocabulary so FTS has realistic term frequencies.
- Smaller presets for PRs: `--preset small` (2k people / 50k interactions, ~30 s to build) and `--preset medium` (20k people / 500k interactions without the 5 GB mbox).

## Benchmark harness

### criterion (micro and query latency)

Benches live in `crates/openstance-core/benches/` and `crates/openstance-import/benches/`:

```rust
fn bench_structured(c: &mut Criterion) {
    let g = Graph::open_read_only(synth_path("heavy/graph.db")).unwrap();
    let qs = load_queries("heavy/queries.toml", Kind::Structured);
    let mut grp = c.benchmark_group("search/structured");
    grp.sample_size(200).measurement_time(Duration::from_secs(10));
    for q in qs.iter().take(20) {
        grp.bench_with_input(BenchmarkId::from_parameter(&q.id), q, |b, q| {
            b.iter(|| g.search(black_box(&q.query)).unwrap())
        });
    }
    grp.finish();
}
```

- `harness = false` benches, criterion with `--save-baseline` / `--baseline` for comparisons.
- Query-set benches report per-query and an aggregate p50/p95 computed by `cargo xtask bench` from criterion's `estimates.json` and raw samples.
- Text-search benches run twice: with the hash embedder (deterministic, PR CI) and with fastembed (nightly and local).

### xtask end-to-end (import, rebuild, first-run embedding, size)

`cargo xtask bench --e2e` runs the release binary as a subprocess against a fresh `OPENSTANCE_HOME`, measures wall clock, and samples peak RSS (`getrusage(RUSAGE_CHILDREN)` on Unix, `GetProcessMemoryInfo` on Windows). Each scenario runs 3 times; the median is reported. Output is JSON (`target/bench/e2e.json`) with the git SHA, machine fingerprint (CPU model, core count, RAM, OS) and the numbers.

### hyperfine (CLI wall clock)

```sh
hyperfine --warmup 3 -N \
  'target/release/openstance --home target/synth/heavy search --company "Lumen" --met --json' \
  'target/release/openstance --home target/synth/heavy who 1234 --json'
```

Informational; used to make sure process startup (config load, DB open, migration check) stays under ~30 ms.

## Regression budget in CI

Shared GitHub runners are noisy, so CI uses two kinds of checks:

1. **Relative budget on PRs** (bench job, see [ci-and-release.md](ci-and-release.md#benchmarks-conditional)). The job builds `main` and the PR head on the same runner, generates the `medium` synthetic graph (cached), and runs criterion benches for both. A benchmark fails the check if the PR is slower than `main` by more than:

   | Group | Allowed slowdown |
   |---|---|
   | `search/*`, `who/*`, `how_connected/*` | 10% on p95 |
   | `import/linkedin_5k` | 10% |
   | `rebuild/*` (medium preset) | 15% |
   | Peak RSS on `import/mbox` (1 GB variant on PRs) | 10% and never above 500 MB |

   Changes within the noise band (criterion reports "no change" at p = 0.05) always pass. A PR that intentionally trades speed for correctness adds the `perf-accepted` label with a sentence of justification; a maintainer must approve it.

2. **Absolute ceilings everywhere**, with headroom for slow runners. These fail any PR outright:

   | Check | CI ceiling (medium preset, GitHub runner) |
   |---|---|
   | structured search p95 | 100 ms |
   | text search p95 (hash embedder) | 150 ms |
   | `who` / `how_connected` p95 | 40 ms |
   | release binary size, any target | 40 MB |

3. **Nightly full run** on the `heavy` preset with fastembed, all targets in the table above, results appended to a history file on the `gh-pages` branch (`bench/history.json`) and charted. A target miss opens an issue. Before each release, the full suite is run on an actual M2 machine by a maintainer and the numbers are pasted into the release PR; launch claims (README, Show HN) quote only these M2 numbers.

## Design rules that protect the targets

- Importers stream: mbox is parsed message-by-message from a buffered reader with header-only parsing by default; nothing holds the whole file in memory. Batch inserts of ~10k rows per prepared-statement loop inside one transaction per import.
- Rebuild runs entirely inside SQLite where possible (set-based `INSERT ... SELECT` for ties from `interaction_person` self-joins, `n_shared` counts and `last_at` maxima), with Rust only for resolution rules that need normalization logic.
- Scores are cached on `person.score` with components stored, so search never computes them at query time. Recency uses `R = exp(-ln(2) * days_since_last / half_life_days)` (default half-life 180 days); because R depends on "now", cached scores are refreshed lazily when older than 24 h (one pass over 20k rows, < 1 s) rather than at query time.
- Bulk people (`person.is_bulk = 1`) are filtered with `is_bulk = 0` in the candidate SQL (and the FTS/vector candidate lists are post-filtered) so they never enter result sets by default.
- Indexes from the spec are required and asserted by a test: `identity(kind, value)` unique, `interaction(at)`, `interaction_person(person_id)`, `tie(a)`, `tie(b)`, `person(score DESC)`, `person(last_contact DESC)`.
- Vector search stays brute force (sqlite-vec `vec0`) at this scale; an ANN index is not justified below ~200k vectors. Revisit only if a benchmark says so.
- Embedding is batched (fastembed batch size 256) and only re-embeds people whose embedding text changed (hash of `display_name + title + company + bio + top subjects` stored alongside).

## When a target is missed

1. Reproduce locally with `cargo xtask bench --only <id> --preset heavy`.
2. Use the component budgets above to locate the stage; `OPENSTANCE_LOG=openstance_core=debug` emits per-stage timings in search spans; `cargo flamegraph` / `samply` for CPU, `EXPLAIN QUERY PLAN` for SQL.
3. Fix, or if the target itself is wrong (for example a new pipeline stage the spec did not foresee), propose a revised target in a PR that updates this document and records why. Targets are not raised silently.
