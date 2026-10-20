# Testing strategy

This document defines how openstance is tested: what fixtures exist, which test tiers run where, what each tier asserts, and the coverage bar a change must meet before merge. It implements the "Testing" section of the original tech spec and extends it with importer fuzzing, property tests and an explicit CI/local split.

Related: [performance-targets.md](performance-targets.md) (benchmarks), [ci-and-release.md](ci-and-release.md) (where each tier runs), [conventions.md](conventions.md) (no real contact data), [../tech-spec/scoring.md](../tech-spec/scoring.md), [../tech-spec/search.md](../tech-spec/search.md), [../tech-spec/entity-resolution.md](../tech-spec/entity-resolution.md), [../tech-spec/playbooks.md](../tech-spec/playbooks.md).

## Principles

1. **CI never needs a model, a network, or a secret.** Every CI tier runs with `parser = "rules"`, `reranker = "none"`, `enrich.backend = "none"` (or `MockEnricher`), and either the Null embedder or a deterministic hash embedder. Model-backed tiers run locally and are opt-in.
2. **The graph is a pure function of sources plus overrides.** Most correctness tests reduce to "import these fixtures, apply these overrides, assert this graph". That is also what the rebuild property test checks.
3. **Fixtures are synthetic and fictional only.** No real names, emails, phone numbers, handles or company-internal data ever enter the repo, including in bug reports turned into regression tests. See [Fixture rules](#fixture-rules).
4. **Golden files change on purpose.** Scoring and search outputs are pinned. Changing a weight, a formula or a tokenizer must update the golden file in the same PR, and the diff is reviewed like code.
5. **Every egress path is tested without egress.** Playbooks and enrichment run against `MockEnricher`, which records requests. Tests assert on what *would* have left the machine.

## Test tiers

| Tier | What | Tool | Runs in CI | Runtime budget |
|---|---|---|---|---|
| Unit | Pure functions: normalization, scoring components, RuleParser, fusion math, override resolution, config parsing | `cargo test` (`#[cfg(test)] mod tests`) | yes, every push | < 30 s total |
| Integration | One crate end to end against a temp SQLite file: import -> resolve -> score -> search | `cargo test` in `crates/*/tests/` | yes, every push | < 90 s total |
| Golden | Scoring for 10 hand-computed people; `who` and `search --json` output shapes | `insta` snapshots + hand-computed TOML | yes, every push | < 5 s |
| Search evals (rules tier) | 50 queries, expected top-5 ids, rules parser + no reranker + FTS5 (+ hash embedder) | `cargo xtask eval --tier rules` | yes, every push | < 20 s |
| Search evals (model tier) | Same 50 queries with fastembed, then with Ollama parser/reranker | `cargo xtask eval --tier fastembed` / `--tier ollama` | fastembed: nightly only; ollama: local only | minutes |
| Property | Rebuild idempotence, override algebra, resolution monotonicity, recency bounds | `proptest` | yes (reduced cases); nightly (full cases) | < 60 s PR, < 15 min nightly |
| Playbook | Each playbook against `MockEnricher`: quotes, budgets, `no_enrich`, `do_not_share`, `allow_egress=false` | `cargo test -p openstance-core playbook` | yes | < 10 s |
| Interface | CLI (`assert_cmd`), MCP (rmcp client over in-memory transport), HTTP (`axum` `Router` + `tower::ServiceExt::oneshot`) all return the same JSON shape | `cargo test -p openstance -p openstance-mcp -p openstance-api` | yes | < 30 s |
| Fuzz | Importer parsers on arbitrary bytes | `cargo fuzz` (libFuzzer) | smoke (60 s per target) nightly; long runs local/OSS-Fuzz later | n/a |
| Benchmarks | Performance targets on synthetic 20k / 500k graph | `criterion` + `cargo xtask bench` | nightly + on `perf`-labelled PRs | see [performance-targets.md](performance-targets.md) |
| UI | Optional web UI component + API-contract tests | `vitest` + Playwright smoke | yes when `ui/` changes | < 2 min |

## Test layout

```
openstance/
  crates/
    openstance-core/
      src/**                      # unit tests inline: #[cfg(test)] mod tests
      tests/
        resolution.rs             # fixture overlap counts, flagged merges
        scoring_golden.rs         # 10-person golden
        search_rules.rs           # structured + FTS5 queries
        rebuild_props.rs          # proptest: idempotence, override algebra
        playbooks.rs              # MockEnricher tests
        migrations.rs             # every migration from v1 -> head on a seeded db
      benches/
        search.rs  rebuild.rs  who.rs
    openstance-import/
      tests/
        mbox.rs  linkedin_csv.rs  vcf.rs  ics.rs  x_archive.rs  detect.rs
      fuzz/                       # cargo-fuzz workspace (excluded from main workspace)
        fuzz_targets/
          mbox.rs  linkedin_csv.rs  vcf.rs  ics.rs  x_archive.rs
        corpus/<target>/          # seed corpus = fixture files (synthetic)
      benches/
        mbox_stream.rs  linkedin_csv.rs
    openstance-enrich/
      src/mock.rs                 # MockEnricher (behind `test-util` feature)
      tests/treg_contract.rs      # request/response shape against recorded JSON, no network
    openstance-model/
      src/hash_embedder.rs        # deterministic test embedder (behind `test-util`)
      tests/fastembed.rs          # #[ignore] unless OPENSTANCE_TEST_MODELS=1
    openstance-mcp/tests/tools.rs
    openstance-api/tests/http.rs
    openstance/tests/cli.rs       # assert_cmd + insta snapshots of --json output
  fixtures/
    README.md                     # what each file contains, the overlap table below
    persona/                      # the fictional cast (people.toml) used to generate everything
    mbox/  linkedin/  x_archive/  vcf/  ics/  csv/
    golden/
      scoring.toml                # 10 people, hand-computed components and S
      overrides.toml              # override set used by golden + property tests
    evals/
      queries.toml                # 50 queries with expected top-5 ids
      baseline-rules.json         # last accepted metrics for the rules tier
      baseline-fastembed.json
    legacy-v0.1/                  # demo.js people converted to CSV (prototype fixtures)
  xtask/                          # cargo xtask: gen-fixtures, eval, bench, fuzz-smoke
  ui/src/**/*.test.ts
```

Snapshot files (`insta`) live next to the tests in `snapshots/` directories and are committed.

## Fixtures

### The fictional cast

All fixtures are generated from a single cast file, `fixtures/persona/people.toml`, so overlaps are known by construction. The cast is roughly 60 fictional people at fictional companies (domains under `example.com`, `example.org`, `example.net` and the reserved `.test` / `.example` TLDs only; phone numbers from the fictional ranges, e.g. `+1-555-01xx`, `+44 7700 900xxx`).

Each cast entry declares which sources it appears in and under which handles:

```toml
[[person]]
key = "maya_r"
name = "Maya Rao"
company = "Lumen Labs"           # fictional
title = "Founder"
location = "Bengaluru"
emails = ["maya@lumenlabs.example", "maya.rao@gmail.test"]
linkedin = "maya-rao-lumen"
x = "mayabuilds"
phone = "+15555550142"
sources = ["mbox", "linkedin", "vcf", "ics", "x_archive"]
expect = { merges = "auto", flagged = false }
```

`cargo xtask gen-fixtures --cast fixtures/persona/people.toml` writes the per-source files. Generated files are committed (reviewers must be able to read them) and CI fails if regenerating produces a diff.

The v0.1 prototype's fictional demo network (`dist/demo.js`) is converted once into `fixtures/legacy-v0.1/` as a CSV source so the Rust importers and search can be compared against prototype behavior; see [../prototype-v0.1.md](../prototype-v0.1.md).

### Required overlap scenarios

The resolution integration test asserts exact counts. The fixture set must contain at least these cases, each tagged in the cast with `expect`:

| Scenario | Sources | Expected outcome |
|---|---|---|
| Same email across mbox, vcf, ics | 3 | one person, auto-merge, confidence 1.0 |
| Gmail dots and plus aliasing (`a.b@gmail.test`, `ab+x@gmail.test`) | mbox | one person, 0.98 (gmail rules apply to the configured gmail domains; tests configure `gmail.test`) |
| Corporate plus-addressing | mbox | one person, 0.98 |
| LinkedIn connection with no email, same name + same company as an mbox contact | linkedin + mbox | merged, flagged (0.85) |
| Same name, appear on the same thread, different emails | mbox | merged, flagged (0.75) |
| Rare identical name, nothing else shared | linkedin + vcf | **not** merged; one suggestion in `review` (0.5) |
| Common identical name ("Alex Kim" x2), different companies | linkedin + mbox | never merged, no suggestion |
| Jaro-Winkler near-miss ("Jon Smyth" / "John Smith") | vcf + linkedin | suggestion only, never auto-merged |
| X handle with only a name match | x_archive + linkedin | suggestion only |
| Phone match vcf <-> whatsapp-style csv | vcf + csv | one person, 1.0 (E.164 normalization) |
| Newsletter sender: 60 messages, zero replies, `List-Unsubscribe` | mbox | `is_bulk = 1`, score 0, excluded from default search |
| Override `merged_into` across two unrelated people | overrides.toml | merged regardless of rules |
| Override `split_from` on an exact-email match | overrides.toml | kept apart after rebuild |
| Same file imported twice | any | second import is a no-op (`source.file_hash`) |
| Same Message-ID in two mbox files | mbox | one interaction (`UNIQUE(source_id, external_id)` per source; cross-source dedupe asserted separately) |
| Unicode names (NFKC, combining marks, emoji in display name, honorifics) | vcf + linkedin | normalized and merged as expected |
| `do_not_share = 1` and `no_enrich = 1` people | overrides.toml | excluded from views; skipped by enrichment |

The resolution test then asserts the totals: `persons == N`, `flagged_merges == F`, `suggestions == S`, `bulk == B`, with N/F/S/B recorded in `fixtures/README.md`.

### Fixture rules

- Fictional people only. Names are picked to be plausible but not matching any notable real person; domains use reserved TLDs/`example.*`.
- Never paste a real export, even "anonymized". If a user bug report needs a reproduction, rebuild the shape synthetically (same headers, same encoding quirk) under the cast.
- Fixture files stay small: each source < 200 KB. Large inputs come from the generator at test time, not from the repo.
- A pre-commit hook and a CI job scan `fixtures/` and test sources for email domains outside the allowlist (`example.*`, `*.test`, `*.example`, `*.invalid`) and for phone numbers outside the fictional ranges. See [conventions.md](conventions.md#no-real-contact-data).

### Synthetic scale generator

`cargo xtask gen-graph --people 20000 --interactions 500000 --ties 100000 --seed 42 --out target/synth/heavy` produces:

- a synthetic mbox (headers only, ~5 GB variant with `--mbox-bytes 5G` for the import benchmark),
- a LinkedIn `Connections.csv` with 5k rows,
- a vcf and ics with realistic overlap ratios (about 30% of people in two or more sources),
- a ready-built `graph.db` for search/who benchmarks so benches do not pay import cost.

Distributions: interactions per person follow a Zipf-like distribution (a few hundred heavy contacts, a long tail); 3% bulk senders; timestamps spread over 6 years with recency skew. The generator is deterministic for a given seed; benchmarks and the "heavy" integration test always pass `--seed`.

## Unit tests

Unit tests live inline. Required coverage by module:

- **Normalization**: NFKC, lowercase, honorific and emoji stripping, whitespace collapse; company suffix stripping ("Inc", "Ltd", "Pvt"); email normalization including gmail rules; E.164 phone parsing; LinkedIn slug canonicalization from every URL form we have seen (with and without `www`, trailing slash, locale subpaths, query strings).
- **Scoring components**, each in [0, 1]:
  - `R = exp(-ln(2) * days_since_last / half_life_days)`; assert `R(0) = 1`, `R(180) = 0.5 +- 1e-9` with the default half-life, `R(360) = 0.25`, monotone decreasing, and never negative for future timestamps (clamp `days_since_last` at 0).
  - `F = 1 - exp(-n/20)`, `M = 1 - exp(-n_meetings/3)`, `C` capped at 1 with the "thread they started counts double" rule, `B = channels_seen / 4`.
  - Preset weights each sum to 1.0; with an empty config the `default` preset applies (R .35 F .20 C .25 M .15 B .05); `founder` is .30/.15/.30/.20/.05 ([scoring](../tech-spec/scoring.md#4-presets)).
  - Bulk people return `S = 0` regardless of components.
- **Override resolution**: scalar fields last-write-wins by `set_at` (ties broken by `id`); tombstone (`value IS NULL`) removes a scalar; multi-valued fields (`tag`, `note`, `merged_into`, `split_from`) accumulate; deleting a multi-valued row removes exactly that value.
- **RuleParser**: table-driven tests (`rstest` cases) mapping text to `Query`, including dates ("since March", "last 6 months" relative to an injected `today`), flags ("met in person", "replied", "lost touch"), company and title extraction, and no-op on unparseable text (whole string becomes `Query.text`).
- **Fusion**: min-max normalization over the candidate set with degenerate cases (one candidate, all-equal scores, FTS-only, vector-only), 0.5/0.5 combination, `(0.5 + 0.5 * score)` lift.
- **Config**: empty file yields all defaults; unknown keys are warnings (errors with `config --strict`) naming the key path; type and range errors are errors; `weights` and an explicitly set `preset` are mutually exclusive.

## Integration tests

Each integration test creates a fresh temp directory (`tempfile::TempDir`), runs `openstance init` equivalents through the library API, imports fixtures, and asserts on the database. Helpers live in `openstance-core`'s `test-util` feature:

```rust
let g = TestGraph::new()                    // temp ~/.openstance, migrations applied
    .import("fixtures/mbox/work.mbox")
    .import("fixtures/linkedin/Connections.csv")
    .overrides("fixtures/golden/overrides.toml")
    .rebuild();
assert_eq!(g.count_persons(), 41);
assert_eq!(g.flagged_merges().len(), 6);
```

Mandatory integration scenarios:

1. Each importer: row counts per table, `source.kind`, idempotent re-import, header-only for mbox unless `import.email_bodies = true`.
2. Resolution totals (table above).
3. Search: structured filters compose; FTS `subjects` column finds attendees of a meeting by its title; bulk people excluded by default and included with `--include-bulk`.
4. `who` returns components that recombine to the stored score within 1e-6.
5. `how_connected` returns at most 3 paths ordered by `my_score(mutual) * tie_weight(mutual, target)`, each with evidence rows drawn from `interaction_person` joins on both people.
6. Migrations: a database at every released schema version (fixtures under `fixtures/db/schema-vN.db`) upgrades to head and passes the same assertions.
7. `person_fts` consistency: after any edit, `SELECT count(*) FROM person_fts` equals `SELECT count(*) FROM person` and each row matches the current person fields (delete+insert maintenance); `openstance rebuild` produces an identical FTS table.

## Golden tests: scoring

`fixtures/golden/scoring.toml` holds 10 people chosen to exercise every component edge: a fresh heavy correspondent, a 2-year-old contact, a meet-only contact, a one-way sender, a thread-starter (double-count rule), a bulk sender, a LinkedIn-only connection, a multi-channel person (B = 1), a person with an override that changes nothing in score, and a person at exactly 180 days (R = 0.5).

```toml
today = 1792540800          # fixed clock injected into scoring
half_life_days = 180
preset = "default"

[[person]]
key = "maya_r"
R = 0.9623
F = 0.7769
C = 1.0
M = 0.4866
B = 0.75
S = 0.8527              # .35R + .20F + .25C + .15M + .05B  (10 days, 30 msgs, 2 meetings, 3 channels)
```

Each value is hand-computed (a spreadsheet or Python snippet in `fixtures/golden/compute_scoring.py` documents the arithmetic; it is not run in CI). The test asserts each component and S to 1e-4 for the default preset, then re-asserts S for all four presets (`founder`, `recruiter`, `seller`, `community`). Changing a formula, weight or default requires editing this file in the same PR; the PR template has a checkbox for it.

## Golden tests: output shapes

`insta` JSON snapshots pin the shapes defined in [../tech-spec/interfaces.md](../tech-spec/interfaces.md) for `search_people`, `get_person`, `how_connected`, `brief`, `quote_enrich`. The same fixture query is run through the CLI (`--json`), the MCP tool and the HTTP endpoint, and all three snapshots must be byte-identical after redacting timestamps. Use `insta::assert_json_snapshot!(value, { ".generated_at" => "[ts]" })`.

## Search evals

### Query set

`fixtures/evals/queries.toml` contains 50 queries over the fixture graph (cast + legacy-v0.1), each with expected top-5 person keys and optional "must not appear" keys:

```toml
[[q]]
id = "q07"
text = "founders in Bengaluru I've met"
expect_top5 = ["maya_r", "dev_s", "anika_p"]   # order-insensitive set; may be < 5
must_not = ["newsletter_bot"]
tags = ["structured", "location", "met"]
```

Mix: 15 pure structured (company/title/location/date flags), 15 free-text over FTS fields including subjects, 10 mixed, 5 "who knows someone at X" (company filter + how_connected), 5 adversarial (bulk senders, common-name collisions, negated notes must not leak).

### Metrics and gates

`cargo xtask eval --tier rules` prints, per tier: **Recall@5** (fraction of expected ids in the top 5, averaged), **MRR** over the first expected id, **must_not violations** (count), and per-tag breakdowns. Results are compared with `fixtures/evals/baseline-<tier>.json`:

- CI fails if `must_not violations > 0`.
- CI fails if Recall@5 drops more than 0.02 or MRR drops more than 0.03 versus the baseline.
- Improvements are accepted by running `cargo xtask eval --tier rules --update-baseline` and committing the new baseline in the same PR.

### Tiers

| Tier | Parser | Embedder | Reranker | Where |
|---|---|---|---|---|
| `rules` | RuleParser | hash embedder (deterministic, no download) or none | none | CI, every push |
| `fastembed` | RuleParser | fastembed bge-small-en-v1.5 | none | nightly CI with cached model; local |
| `ollama` | Ollama 3B-8B instruct | fastembed | fastembed `bge-reranker-base` or Ollama | local only (`OPENSTANCE_EVAL_OLLAMA=http://127.0.0.1:11434`) |
| `api` | Anthropic or OpenAI-compatible | fastembed | API | local only, opt-in, prints a cost quote first |

The `ollama` and `api` tiers are informative: they record metrics into `target/evals/` and are reported in PRs that touch parsing or reranking, but they do not gate merge.

## Property tests

Using `proptest`, with strategies that generate small random graphs (2-40 people, 0-200 interactions, random identity overlaps) and random override sequences:

1. **Rebuild idempotence**: `rebuild(rebuild(db)) == rebuild(db)`, compared via a canonical dump (persons sorted by smallest identity, scores rounded to 1e-9, FTS rows, ties).
2. **Rebuild equals incremental**: importing sources one by one with resolution after each equals importing all and rebuilding once.
3. **Order independence**: permuting source import order yields the same canonical graph.
4. **Override durability**: any override applied before a rebuild is reflected after it; `split_from` is never violated by any later import; `merged_into` survives.
5. **Scalar override algebra**: for a random sequence of sets and tombstones on one field, the effective value equals the last non-superseded write by `(set_at, id)`.
6. **Score bounds**: every component and S in [0, 1]; bulk implies S = 0.
7. **Resolution conservatism**: adding a source can only merge people via rules with confidence >= 0.75 or via overrides; never via name-only rules.

PR CI runs 256 cases per property; nightly runs 10,000 and persists failing seeds to `proptest-regressions/` (committed).

## Playbook tests with MockEnricher

`MockEnricher` (in `openstance-enrich`, feature `test-util`) implements `Enricher`, returns canned `EnrichResult`s keyed by request, returns configurable quotes, and records every `quote` and `enrich` call:

```rust
let mock = MockEnricher::new()
    .quote_each(Profile, MicroUsd(20_000))     // $0.02
    .respond(Profile { name: "Ravi Menon".into(), .. }, fixture_json("enrich/ravi.json"));
let out = path_to(&g, &mock, json!({"name": "Ravi Menon", "allow_egress": true, "max_usd": 0.10}))?;
assert_eq!(mock.enrich_calls().len(), 1);
assert!(g.enrich_log().last().unwrap().cost_micro_usd == 20_000);
```

Required assertions for every playbook:

- `allow_egress = false` (the default) returns the plan and quote and makes **zero** `enrich` calls.
- `quote` is always called before `enrich`, and the quoted total is in the outcome receipt.
- A run whose quote exceeds `max_usd` (or `enrich.max_usd_per_run`) aborts before any `enrich` call.
- `no_enrich = 1` people never appear in any request; `do_not_share = 1` people never appear in view exports or in any outbound request.
- Every `enrich` call writes exactly one `enrich_log` row with request kind, backend name and cost.
- Enrichment results land with `source.kind = 'enrich:<name>'` and never overwrite an override value.
- "Enough local data, skip egress" branch: when the target is already in the graph, `path` makes zero calls.

`treg` contract tests replay recorded request/response JSON (synthetic content) through the `treg` backend with an injected HTTP client; no network in CI.

## Importer fuzzing

Each importer exposes a `parse_bytes(&[u8], &mut dyn Sink)` entry point used by its fuzz target. Targets: `mbox`, `linkedin_csv`, `vcf`, `ics`, `x_archive` (JS-wrapped JSON), and later `whatsapp`. Invariants checked by the fuzz harness:

- no panic, no unbounded allocation (harness caps input at 1 MB and asserts RSS growth < 64 MB),
- every emitted `RawInteraction.at` is a valid timestamp, every `RawIdentity.value` is normalized (idempotent under normalization),
- parsing a valid fixture with random bytes appended never loses the records before the corruption point (mbox, csv).

Seed corpora are the synthetic fixture files. Nightly CI runs each target for 60 s (`cargo xtask fuzz-smoke`); crashes become regression tests under `crates/openstance-import/tests/regressions/` with a minimized synthetic input. Fuzzing requires nightly Rust and runs on Linux only.

## Interface tests

- CLI: `assert_cmd` drives the real binary against a temp `OPENSTANCE_HOME`; asserts the exit codes in [interfaces](../tech-spec/interfaces.md#14-exit-codes) (0 success, 1 internal, 2 usage, 3 egress needs approval, 4 budget, 5 not found, 6 ambiguous, 7 view expired, 10 db busy / schema too new), stderr contains no contact data at default log level, `--json` stdout parses and matches the snapshot.
- MCP: an in-process rmcp client lists tools and asserts the exact set: `search_people`, `get_person`, `how_connected`, `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich`. Money-spending tools (`path_to`, `brief`) take `allow_egress` (default false) and `max_usd`; with `allow_egress` false they return a quote and make no enricher call. `quote_enrich` never spends.
- HTTP: requests without the bearer token get 401; the server refuses to bind anything but loopback unless `--host <addr> --i-understand-this-exposes-my-graph` is passed on the command line; requests with a foreign `Host` header get 421; CORS headers present only for the embedded UI origin; `GET /health` needs no token.

## Coverage expectations

Measured with `cargo llvm-cov` on Linux in CI (report uploaded, not a hard gate on the total):

| Area | Line coverage floor (gating) |
|---|---|
| `openstance-core` resolution, scoring, overrides, search fusion | 90% |
| `openstance-import` parsers | 85% |
| `openstance-enrich` budget/quote/log enforcement (core side) | 95% |
| `openstance-mcp`, `openstance-api`, CLI wiring | 70% |
| `openstance-model` provider adapters | best effort (network-bound code is behind traits) |

The gate is per-crate via `cargo xtask coverage --check`, which reads floors from `xtask/coverage.toml`. A PR may lower a floor only with a written reason in the PR description.

Beyond percentages, the review checklist requires: every bug fix adds a regression test; every new importer adds a fixture, a detect test, an integration test and a fuzz target; every new playbook adds MockEnricher tests for all required assertions above.

## Running tests locally

```sh
just test                      # cargo nextest run --workspace (unit + integration + golden + playbook)
just eval                      # cargo xtask eval --tier rules
just eval-models               # fastembed tier (downloads model once to the cache)
OPENSTANCE_EVAL_OLLAMA=http://127.0.0.1:11434 cargo xtask eval --tier ollama
just props-long                # PROPTEST_CASES=10000 cargo test --release -p openstance-core --test rebuild_props
just fuzz mbox 300             # 5-minute fuzz run on the mbox target (nightly toolchain)
cargo insta review             # accept/reject snapshot changes
```

See [dev-setup.md](dev-setup.md) for toolchain installation and the `justfile`.
