# openstance docs

openstance is a local-first relationship graph. It imports exports you already have (Gmail Takeout mbox, LinkedIn `Connections.csv`, vCards, later calendars and X archives), resolves them into one person per human, scores every relationship with a deterministic, explained formula, and answers "who do I know who..." questions from the CLI, from AI agents over MCP, and from a local HTTP API. Everything lives in one SQLite file (`~/.openstance/graph.db`). It works offline with no API keys. Nothing leaves the machine unless you approve a quoted, logged enrichment call or export an encrypted view.

- Binary `openstance` (alias `ost`). Rust workspace: `openstance-core`, `openstance-import`, `openstance-enrich`, `openstance-model`, `openstance-mcp`, `openstance-api`, `openstance` (CLI).
- Storage: SQLite + FTS5 + sqlite-vec. Local embeddings: fastembed `bge-small-en-v1.5` (384d). Local API: `127.0.0.1:7411`. Shared views: age-encrypted `*.ost` files.
- "openstance" is the working name. Renaming (the overview suggests "kith") is an open decision: [ADR-0001](decisions/0001-product-name.md).
- A browser prototype (v0.1) already exists. It is Phase 0: reference material and a source of fixtures, not the product ([prototype-v0.1.md](prototype-v0.1.md)).

## How these docs are organised

| Folder | What it answers | Authority |
|---|---|---|
| [product/](product/vision.md) | Why build this, for whom, against what, and how it could make money | Positioning and claims |
| [architecture/](architecture/overview.md) | How the system is shaped: crates, data flow, trust boundaries | Structure and dependency rules |
| [tech-spec/](tech-spec/data-model.md) | Exactly what to build: schemas, algorithms, signatures, JSON, config | Build-ready contracts. If two docs disagree, the tech-spec doc that owns the topic wins |
| [plan/](plan/roadmap.md) | In what order to build it: phases, tasks, acceptance criteria, launch | Scope and sequencing |
| [engineering/](engineering/dev-setup.md) | How to work on it: setup, conventions, tests, CI, performance budgets | Process |
| [decisions/](decisions/README.md) | Why the big choices were made (ADRs) | An accepted ADR wins over any other doc until it is superseded |

Ownership of shared topics: the schema belongs to [data-model.md](tech-spec/data-model.md); JSON shapes, CLI commands, MCP tools, HTTP routes and exit codes belong to [interfaces.md](tech-spec/interfaces.md); config keys belong to [configuration.md](tech-spec/configuration.md); score and tie formulas belong to [scoring.md](tech-spec/scoring.md). Fixes to the original tech spec PDF are listed in [ADR-0012](decisions/0012-spec-corrections.md).

## Every doc

### Product

| Doc | Description |
|---|---|
| [product/vision.md](product/vision.md) | Problem, product principles (local-first, explained, agent-native, egress off), what openstance is and is not |
| [product/personas-and-use-cases.md](product/personas-and-use-cases.md) | Founder, recruiter, seller, community and agent personas, with concrete commands and the use-case matrix by phase |
| [product/competitive-analysis.md](product/competitive-analysis.md) | Factual comparison with Happenstance and adjacent tools, each claim labelled verified or unconfirmed, and where competitors still win |
| [product/github-inspiration.md](product/github-inspiration.md) | Open-source references (Lean Intros, Orbit, Relationship Substrate, Twenty, Monica, Relaticle), licence caveats, what we adapt, and a gap analysis against the code |
| [product/pricing-and-cloud.md](product/pricing-and-cloud.md) | Free local core, bring-your-own-key enrichment, an indicative optional cloud tier and the rules it must follow |

### Architecture

| Doc | Description |
|---|---|
| [architecture/overview.md](architecture/overview.md) | System context, components, the egress gate, on-disk layout, why Rust and SQLite, phase-by-phase delivery |
| [architecture/crates-and-dependencies.md](architecture/crates-and-dependencies.md) | Workspace layout, dependency direction (core never imports a concrete provider), trait seams, cargo features |
| [architecture/data-flow.md](architecture/data-flow.md) | Import, resolve, score, index, search, enrich, view export and rebuild as sequence diagrams |
| [architecture/security-and-threat-model.md](architecture/security-and-threat-model.md) | Assets, trust boundaries, threats and mitigations, egress controls, local API hardening, honest limits |

### Tech spec

| Doc | Description |
|---|---|
| [tech-spec/data-model.md](tech-spec/data-model.md) | Full SQLite DDL (9 tables + FTS5 + vec0 + `meta`), raw/derived/edit classes, stable anchor ids, overrides, migrations, Parquet export |
| [tech-spec/ingestion.md](tech-spec/ingestion.md) | `Importer`/`Sink` traits, detection, dedupe, and per-importer specs (mbox, linkedin_csv, vcf, ics, x_archive, csv, whatsapp, imap) |
| [tech-spec/entity-resolution.md](tech-spec/entity-resolution.md) | Normalization, the ordered merge rules with confidences, review queue, merge/split, incremental equals full rebuild |
| [tech-spec/scoring.md](tech-spec/scoring.md) | R/F/C/M/B components, presets, bulk senders, stored columns, ties and path strength, worked example |
| [tech-spec/search.md](tech-spec/search.md) | `Query` struct, RuleParser, structured filter, FTS5 + vector candidates, fusion, rerank, evidence, result JSON, degradation |
| [tech-spec/models.md](tech-spec/models.md) | `Embedder`/`QueryParser`/`Reranker` traits, fastembed default, Ollama/Anthropic/OpenAI backends, re-indexing, model egress rules |
| [tech-spec/enrichment.md](tech-spec/enrichment.md) | `Enricher` trait, `EgressGate`, quotes and budgets, `enrich_log`, the treg backend, writing results without clobbering edits |
| [tech-spec/playbooks.md](tech-spec/playbooks.md) | `Playbook` trait and the path, brief, accounts, refresh, watch, reconnect and verify playbooks; agent skill files |
| [tech-spec/interfaces.md](tech-spec/interfaces.md) | Shared JSON shapes, the 8 MCP tools, HTTP routes, every CLI command and flag, exit codes |
| [tech-spec/sharing-and-views.md](tech-spec/sharing-and-views.md) | Views as saved queries, the `.ost` file format, age encryption, expiry limits, Phase 5 sync and team federation |
| [tech-spec/configuration.md](tech-spec/configuration.md) | `~/.openstance/` layout, precedence and env vars, the complete `config.toml` with every default |

### Plan

| Doc | Description |
|---|---|
| [plan/roadmap.md](plan/roadmap.md) | Phases 0 to 5, timeline, gates (public MVP, park criterion, cloud decision), what is out of scope per phase, and the research backlog (imports, warm paths, intros) |
| [plan/phase-1.md](plan/phase-1.md) | Week 1: schema, mbox/LinkedIn/vcf importers, resolution and review, scoring, RuleParser + FTS5 search, usable CLI |
| [plan/phase-2.md](plan/phase-2.md) | Week 2: embeddings and fusion, `who`, `how`, MCP server, 60-second demo, Show HN (the public MVP) |
| [plan/phase-3.md](plan/phase-3.md) | Week 3: Enricher trait with Null and treg, path and brief, encrypted views, ics and X archive importers |
| [plan/phase-4.md](plan/phase-4.md) | Week 4: HTTP API and minimal UI, remaining playbooks, model backends, cargo-dist releases (v1.0) |
| [plan/phase-5-and-beyond.md](plan/phase-5-and-beyond.md) | Post-launch, gated on traction: sync, team federation, more importers, optional OAuth, relationship memory, provider-backed research, cloud decision |
| [plan/launch.md](plan/launch.md) | Positioning, Show HN copy, channels, README checklist, metrics, and the park criterion thresholds |

### Engineering

| Doc | Description |
|---|---|
| [engineering/dev-setup.md](engineering/dev-setup.md) | Toolchain (MSRV 1.92), repo layout, running against a throwaway `OPENSTANCE_HOME`, UI dev loop, env vars |
| [engineering/conventions.md](engineering/conventions.md) | Errors, logging without contact data, SQL and migration rules, feature flags, commit style |
| [engineering/testing-strategy.md](engineering/testing-strategy.md) | Synthetic fixtures, unit/integration/golden/property/fuzz tests, search eval gates, interface contract tests |
| [engineering/ci-and-release.md](engineering/ci-and-release.md) | CI pipeline, dependency rules enforced in CI, cargo-dist release targets and installers, versioning |
| [engineering/performance-targets.md](engineering/performance-targets.md) | Latency, import, rebuild, memory and size budgets, how they are measured and enforced on PRs |

### Decisions and reference

| Doc | Description |
|---|---|
| [decisions/README.md](decisions/README.md) | ADR index (0001 to 0014, including the proposed [ADR-0014](decisions/0014-evidence-classes-and-claim-provenance.md) on evidence classes) and pending decisions |
| [risks-and-open-questions.md](risks-and-open-questions.md) | Risk register (R1 to R30) and open questions, reviewed at every phase gate |
| [glossary.md](glossary.md) | Terms used across the docs (anchor id, override, tombstone, tie, view, egress, ...) |
| [prototype-v0.1.md](prototype-v0.1.md) | What the v0.1 browser prototype does, what carries over to the Rust core, and where it lives |

## Start here

### (a) New contributor

1. [product/vision.md](product/vision.md): what we are building and the principles that decide trade-offs.
2. [architecture/overview.md](architecture/overview.md), then [architecture/crates-and-dependencies.md](architecture/crates-and-dependencies.md): the shape of the system and the rules for where code goes.
3. [decisions/README.md](decisions/README.md): skim the ADR titles; read [ADR-0012](decisions/0012-spec-corrections.md) so the differences from the original spec PDF do not surprise you.
4. [engineering/dev-setup.md](engineering/dev-setup.md) and [engineering/conventions.md](engineering/conventions.md): get a build running against a throwaway data dir.
5. [engineering/testing-strategy.md](engineering/testing-strategy.md): fixtures are synthetic; never commit real contact data.
6. [plan/roadmap.md](plan/roadmap.md): find the current phase, then pick a task from that phase's doc.
7. Keep [glossary.md](glossary.md) and [risks-and-open-questions.md](risks-and-open-questions.md) open for reference.

### (b) Building Phase 1

1. [plan/phase-1.md](plan/phase-1.md): the task list, acceptance criteria and cut order.
2. [tech-spec/data-model.md](tech-spec/data-model.md): migration `0001_init.sql`, the raw/derived/edit rules, anchor ids and overrides.
3. [tech-spec/ingestion.md](tech-spec/ingestion.md): the `Importer`/`Sink` traits and the mbox, linkedin_csv and vcf importers.
4. [tech-spec/entity-resolution.md](tech-spec/entity-resolution.md): rules 1 to 6, the review queue, merge and split.
5. [tech-spec/scoring.md](tech-spec/scoring.md): components, presets, bulk detection, ties.
6. [tech-spec/search.md](tech-spec/search.md): sections 2 to 6 and 8 to 10 (RuleParser, structured filter, FTS5, evidence). Vectors and fusion are Phase 2.
7. [tech-spec/interfaces.md](tech-spec/interfaces.md) sections 1 and 4 to 6 (JSON shapes and the Phase 1 CLI), and [tech-spec/configuration.md](tech-spec/configuration.md) for defaults.
8. [engineering/testing-strategy.md](engineering/testing-strategy.md) and [engineering/performance-targets.md](engineering/performance-targets.md): the tests and budgets Phase 1 must pass.
