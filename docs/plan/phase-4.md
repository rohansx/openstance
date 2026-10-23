# Phase 4 (Week 4): local API and UI, the remaining playbooks, model adapters, v1.0

**Timebox:** 5 working days. **Release:** `v1.0.0` with prebuilt binaries. **Previous:** [Phase 3](phase-3.md). **Next:** [Phase 5 and beyond](phase-5-and-beyond.md) (gated). **Overview:** [roadmap](roadmap.md).

## Goal

This phase finishes the v1 feature set and makes it installable with one command.

When it ends:

- `openstance serve` exposes the same JSON over a localhost HTTP API, with a minimal embedded web UI for browsing, editing, merging, and reviewing.
- The follow-through playbooks are in: `watch`, `reconnect`, `accounts`, `refresh`, and `verify`.
- Users can choose a local or hosted model for query parsing and reranking.
- `curl | sh`, Homebrew, and `cargo binstall` install a binary smaller than 40 MB.

## Scope

In scope:

- `openstance-api` (axum): bearer-token HTTP API on `127.0.0.1:7411`, MCP over HTTP on the same server, and embedded static UI. See [interfaces](../tech-spec/interfaces.md).
- `ui/`: TypeScript + Vite. Pages for search, person (edit/merge/split), review queue, views, and egress log.
- Playbooks: `watch`, `reconnect`, `accounts`, `refresh`, `verify`. See [playbooks](../tech-spec/playbooks.md).
- `openstance-model` adapters: Ollama, Anthropic, and OpenAI-compatible `QueryParser` and `Reranker`; the fastembed cross-encoder reranker; alternative embedders (OpenAI `text-embedding-3-small`, Ollama `nomic-embed-text`). See [models](../tech-spec/models.md).
- Second-tier search eval with Ollama (local only).
- `export --format parquet`, behind a cargo feature so the default binary stays small.
- cargo-dist release pipeline. See [CI and release](../engineering/ci-and-release.md).

Out of scope: sync, federation, OAuth, live IMAP, the Phase 5 importers, any hosted service, a Tauri desktop shell, and mobile. See the [roadmap](roadmap.md#what-is-out-of-scope-per-phase).

---

## Task breakdown

### `openstance-api`

- [x] **P4-API-01 axum server skeleton.** `openstance serve [--port 7411]` binds **only** to `127.0.0.1`, and refuses a non-loopback `--host` unless `--i-understand-this-exposes-my-graph` is also given on the command line (config alone is not enough), which prints a warning. It runs tokio inside this crate only. Core calls run on `spawn_blocking` with a connection pool (one writer, N readers).
  - *Acceptance:* `GET /health` returns `{ "ok": true, "version": ..., "schema_version": ... }`. Binding to a non-loopback address without the flag fails.
- [x] **P4-API-02 Bearer token.** A random 256-bit token is generated at `init` (or on the first `serve` for older installs) and stored at `~/.openstance/api.token` with mode 0600. Every route except `/health` needs `Authorization: Bearer <token>`. No cookies.
  - *Acceptance:* a missing or wrong token returns 401, and the comparison runs in constant time. The token never appears in logs.
- [x] **P4-API-03 Routes.** `GET /people?q=&company=&title=&since=&met=&replied=&limit=` (search), `GET /people/{id}` (who), `POST /playbooks/{name}` (body = playbook input including `allow_egress` and `max_usd`), `POST /edits` (override operations, merge, split), `GET /views`, `POST /import` (multipart, streamed to a temp file and then run through the normal importer), `GET /health`. The JSON is the same as CLI `--json` and MCP.
  - *Acceptance:* contract tests run the same fixtures through CLI, MCP, and HTTP and diff the JSON (it must match byte for byte, apart from the transport envelope). `POST /playbooks/path` with the default `allow_egress` returns the quote only.
- [x] **P4-API-04 CORS and origin policy.** CORS is off: the embedded UI is same-origin and needs none, and only origins listed in `serve.cors_origins` (empty by default; Vite dev server during development) get CORS headers. `Host` header validation defends against DNS rebinding.
  - *Acceptance:* tests show that a request with `Origin: https://evil.example` is refused, and that a `Host: attacker.example` header is refused. See [security and threat model](../architecture/security-and-threat-model.md).
- [x] **P4-API-05 MCP over HTTP.** Serve the same 8 tools through rmcp's streamable HTTP transport at `/mcp`, behind the same bearer token, for local clients that speak HTTP MCP. Remote connectors (claude.ai, ChatGPT) cannot reach a loopback address. The docs say so plainly and point to the hosted endpoint idea in [pricing and cloud](../product/pricing-and-cloud.md). No tunnel is shipped.
  - *Acceptance:* the MCP inspector over HTTP lists 8 tools with the same schemas as stdio.
  - *Done:* checked by in-process JSON-RPC tests (`tools/list` over HTTP equals `openstance_mcp::schema::tools()`, tool output equals stdio) rather than a manual inspector run. People marked `do_not_share` are removed from `/mcp` results.
- [x] **P4-API-06 Embedded UI serving.** Embed the built `ui/dist` with `rust-embed` (or `include_dir`). The `ui` feature is on by default. `--no-default-features` builds a binary without the UI.
  - *Acceptance:* `openstance serve` prints `http://127.0.0.1:7411/#token=...`. The UI keeps the token in sessionStorage only.

### `ui/` (TypeScript + Vite)

- [x] **P4-UI-01 Scaffold.** Vite + TypeScript with no heavy framework requirement (a small library such as Preact or Solid is fine), and a typed API client generated from or checked against the JSON schemas.
  - *Acceptance:* the bundle is under 300 KB gzipped. `npm run build` output is reproducible in CI.
  - *Done:* Vite + React + TypeScript, the maintainer's choice ([ADR-0013](../decisions/0013-react-vite-web-ui-and-workspace.md)); about 105 KB gzipped, checked by `npm run size` in the CI `node` job. The client types are hand-written from interfaces.md and checked against a running server on the demo dataset.
- [x] **P4-UI-02 Search page.** A query box. The parsed filters appear as editable chips, so the user can rerun without a model. Results show the score bar and evidence lines.
- [x] **P4-UI-03 Person page.** Fields with provenance and inline edit (each edit is an override), tags, notes, `do_not_share` / `no_enrich` toggles, identities, recent interactions, mutuals, and a "how connected" panel.
- [x] **P4-UI-04 Review queue.** Flagged merges and suggestions with confidence and the matching rule. Confirm, split, and bulk actions.
- [x] **P4-UI-05 Views and egress log.** List and export views, and a read-only `enrich_log` table with totals.
- [x] **P4-UI-06 Playbook runner with quote gate.** Shows the plan and quote. The "Spend $0.0xx" button is the only way to send `allow_egress=true`.
  - *Acceptance (UI):* Playwright smoke tests against `serve` on the fixtures: search, edit title and see it persist after a rebuild, confirm a merge, and run `path` (Mock or Null) and see the quote. The UI works at 375 px width. Basic keyboard navigation works.
  - *Done:* `ui/e2e/smoke.spec.ts` (run by `npm run e2e` and the CI `ui-release` job against the release binary). The `path` run uses `enrich.backend = "treg"` with `OPENSTANCE_OFFLINE=1`: the binary has no Mock backend, and the null backend never quotes, so the treg price estimate gives a real non-zero quote while nothing can leave the machine. Also covered: shortlist, introductions, and token enforcement. Beyond the plan, the UI adds a shortlist and an introductions board backed by schema v4 (ADR-0013).

### `openstance-core` (playbooks)

- [x] **P4-CORE-01 `reconnect`.** People with F > 0.5 and R < 0.2, sorted by their past score (score computed as if R = 1), top 10, each with the subject and date of the last interaction. No egress.
  - *Acceptance:* a golden output on the fixtures. Bulk and `merged_into` people are excluded.
  - *Done:* `playbook/reconnect.rs`, from the stored components. Golden `crates/openstance-core/tests/golden/reconnect_fixtures.json` (`now` 2026-10-20): nobody in the Phase 1 fixtures is lapsed at the default thresholds, so the golden also holds a looser run (F > 0.1, R < 0.4) with real rows. Merged-away people have no `person` row.
- [x] **P4-CORE-02 `accounts <companies.txt>`.** For each company: (1) current people in the graph (company match after normalisation), (2) alumni (past company values from older LinkedIn exports and enrichment sources), (3) one hop through ties to people at the company. Output a coverage table (company, current, alumni, one-hop, best path score). No egress.
  - *Acceptance:* a fixture with two LinkedIn exports shows a job change as alumni. CSV and JSON output.
  - *Done:* `playbook/accounts.rs`; `tests/playbooks_linkedin.rs` imports two `Connections.csv` exports and sees the job change as alumni. Also counts a person with no company and a work address at the company domain as current, and an old work address as alumni (`seen_in: "email_domain"`). CLI `accounts <file> [--csv]`.
- [x] **P4-CORE-03 `watch`.** (1) Diff the latest two `linkedin_csv` sources (company and title changes, new and removed connections). (2) Optionally check profiles for the top 100 by score through the enricher, gated by quote and budget. (3) Flag moves into companies on the target list (`watch.targets` file, same format as `accounts`). There is no daemon. The docs give cron and launchd snippets, and scheduled runs are a cloud-tier idea.
  - *Acceptance:* the local diff runs with zero egress. Step 2 respects `no_enrich` and the budget (MockEnricher test). The output lists the changes, each with its evidence source.
  - *Done:* `playbook/watch.rs`. Only `Connections.csv` sources count as exports (`messages.csv` and `Invitations.csv` are also `linkedin_csv`), ordered by import time. Every diff item carries both export rows as evidence; profile-check moves carry their `enrich_log_id`. Cron and launchd snippets are in [playbooks](../tech-spec/playbooks.md#5-watch---job-change-watch).
- [x] **P4-CORE-04 `refresh`.** Pick people with an empty title or company and 5 or more interactions who are not `no_enrich`, get a quote, then enrich in score order until the budget is used up. Results are written as `source.kind = 'enrich:<backend>'`.
  - *Acceptance:* with a $0.05 budget and a mock price of $0.01 per profile, exactly 5 people are enriched. Overrides are untouched.
  - *Done:* `playbook/refresh.rs`; the budget test is in `playbook/tests_p4.rs`, the commit, override and rebuild checks on a real import in `tests/playbooks_commit.rs`.
- [x] **P4-CORE-05 `verify`.** (1) Pick the best email from the identities (by confidence and recency). (2) Verify it (1 egress). (3) Attach the most recent mutual who replied.
  - *Acceptance:* MockEnricher tests for the verified, invalid, and catch-all outcomes. Without egress, it returns the candidate email marked `unverified`.
  - *Done:* `playbook/verify.rs`. The verdict is committed, and a verdict within `playbooks.verify.fresh_days` is reused without a request.
- [ ] **P4-CORE-06 Parquet export** (feature `parquet`). Exports `person`, `identity`, `interaction`, `interaction_person`, and `tie` as Parquet files with stable column names, so the cloud-tier analytics question stays open.
  - *Acceptance:* round-trip through `duckdb` in an integration test (optional job). The default binary size is unaffected.
- [ ] **P4-CORE-07 Skills** `playbooks/{reconnect,accounts,watch,refresh,verify}.md`. They follow the same rules as Phase 3: quote first, then explicit user consent, and no third-party promotion.

### `openstance-model`

- [ ] **P4-MODEL-01 OpenAI-compatible client.** Shared HTTP client for OpenAI-compatible endpoints (this also covers many local servers), Anthropic, and Ollama. Keys come from the environment or config, and are never logged.
- [ ] **P4-MODEL-02 LLM `QueryParser`.** Ollama (for example `qwen2.5:7b`), Anthropic (a Haiku-class model), and OpenAI-compatible (for example `gpt-4o-mini`), with JSON-schema constrained output. The output is validated against `Query`. On a validation failure, timeout, or error, it falls back to `RuleParser` and reports the reason in `query_source.fallback` plus a `warnings` entry ([search](../tech-spec/search.md#12-degradation)).
  - *Acceptance:* tests with recorded responses cover valid output, invalid JSON, a hallucinated field, and a timeout. The parsed `Query` is always shown and can be edited (CLI `--explain`, UI chips, MCP output).
- [ ] **P4-MODEL-03 Rerankers.** The fastembed cross-encoder `bge-reranker-base` (local), and an API reranker that can also write a one-line reason. Only the top 50 candidates are reranked. The evidence lines stay deterministic. Model prose is marked as such in the JSON (`reason_source: "model:<id>"`).
  - *Acceptance:* `reranker = none` gives identical output to Phase 2. The reranked order is stable for the same inputs (temperature 0 or local).
- [ ] **P4-MODEL-04 Alternative embedders.** OpenAI `text-embedding-3-small` and Ollama `nomic-embed-text`. A change in dims triggers the re-index flow from P2-MODEL-02.
- [ ] **P4-MODEL-05 Hosted-model egress disclosure.** Any hosted model backend (Anthropic, OpenAI, a remote OpenAI-compatible URL) sends query text and candidate profile fields off the machine. It is configured explicitly, never enabled by default, and the first run prints exactly which fields are sent. Each call is recorded in `enrich_log` with a `model:<provider>` backend so the audit trail stays complete. Confirm the final shape in [models](../tech-spec/models.md) and [enrichment](../tech-spec/enrichment.md).
  - *Acceptance:* with the default config, the network-blocked integration test still passes for `search`.
- [ ] **P4-MODEL-06 Eval tier 2.** `cargo xtask eval --tier ollama` runs the 50-query eval with the Ollama parser and reranker and writes a markdown report. It does not run in CI.

### `openstance` (CLI) and release (`ci`)

- [x] **P4-CLI-01 Commands.** `serve`, `watch`, `reconnect`, `accounts <file>`, `refresh [--max-usd] [--yes]`, `verify <person> [--yes]` (egress approval works as in [interfaces](../tech-spec/interfaces.md#7-cli-playbooks)), and `export --format parquet` (when the feature is built).
  - *Done:* `crates/openstance/src/followup.rs`, tested in `tests/cli_playbooks.rs` (verify through the treg-shaped test server: exit 3 with the quote, then `--yes`). `export --format parquet` waits for P4-CORE-06. `POST /playbooks/{name}` serves all seven playbooks. MCP keeps its eight tools ([playbooks](../tech-spec/playbooks.md): only `path` and `brief` are MCP tools).
- [ ] **P4-CI-01 cargo-dist.** Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc` (best effort, can ship later). Installers: shell, PowerShell, Homebrew tap, and cargo-binstall metadata. The `ost` alias is installed alongside.
  - *Acceptance:* a tag push builds and publishes a GitHub Release with checksums. Each binary is under 40 MB without models. A fresh macOS and Linux VM installs with one command and runs the Phase 2 demo.
- [ ] **P4-CI-02 Release checklist** in [CI and release](../engineering/ci-and-release.md): the perf gates, the manual live treg `path` run, a review of the MCP schema snapshots, the network-blocked test, and the changelog.
- [ ] **P4-CI-03 crates.io.** Publish the library crates (once the name decision in ADR-0001 is settled). Until then, do not publish to crates.io, so that a rename does not leave orphaned crate names behind.

### Parallelisation

Streams: (1) API + UI (API-*, UI-*), (2) playbooks (CORE-01..05, 07), (3) model adapters (MODEL-*), (4) release (CI-*). Stream 4 should start on day 1, because platform build problems with ONNX runtime and sqlite-vec are the most likely thing to delay the release.

---

## Deliverables

- `v1.0.0` with prebuilt binaries and installers (shell, Homebrew, binstall).
- `openstance serve` with the HTTP API, MCP over HTTP, and the embedded UI.
- Playbooks: `watch`, `reconnect`, `accounts`, `refresh`, `verify`, plus their skills.
- Model adapters for Ollama, Anthropic, and OpenAI-compatible endpoints, plus the local cross-encoder reranker.
- Parquet export (feature-gated).
- v1.0 announcement ([launch.md](launch.md#post-launch-cadence)).

## Demo script (Phase 4, about 2 minutes)

```sh
curl -LsSf https://github.com/<org>/openstance/releases/latest/download/openstance-installer.sh | sh
ost init --me you@example.com && ost import ~/exports/*
ost reconnect                         # 10 people you used to talk to, with the last subject
ost accounts targets.txt              # warm-coverage table, $0
ost serve                             # opens http://127.0.0.1:7411/#token=...
#  UI: search "devtools investors" -> edit a title -> confirm a flagged merge -> run path (quote shown)
# switch the parser to a local model:
#   edit ~/.openstance/config.toml:  [model] parser = "ollama:qwen2.5:7b"
ost config | grep parser
ost search "people I lost touch with who are now at a payments company" --explain
```

## Exit criteria

1. All P4 tasks are closed or moved with a link.
2. Contract tests show identical JSON from CLI, MCP stdio, MCP HTTP, and HTTP API for every shared operation.
3. Security tests for the API pass: loopback only, token required, CORS and Host checks. The security review checklist in [security and threat model](../architecture/security-and-threat-model.md) is signed off.
4. All performance targets in [performance targets](../engineering/performance-targets.md) are met on the release build, including the binary size under 40 MB.
5. The installer works on clean macOS (arm64) and Linux (x86_64) and runs the Phase 2 demo.
6. The default config still makes zero network calls (the network-blocked integration test passes), except for the one-time embedding model download, which is documented.
7. The README and docs reflect v1.0. The changelog is complete.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cross-platform builds (ort / ONNX, sqlite-vec, Windows) | high | high | Start CI-01 on day 1. Windows is best effort. Fall back to a build without the embedder for targets that fail |
| A local HTTP server opens a browser attack surface | medium | high | Loopback only, bearer token, CORS allow-list, Host check, no cookies. Covered by the threat model |
| UI work grows past the week | high | medium | The UI is minimal by design (5 pages). The CLI and MCP are the product. The UI can slip to a 1.1 without blocking 1.0 |
| LLM parser output drifts or breaks the query | medium | medium | Schema validation, rules fallback, and filters always shown and editable |
| Hosted model backends quietly weaken the privacy story | medium | high | Explicit opt-in, first-run disclosure, `enrich_log` rows (P4-MODEL-05) |
| Name decision still open at v1.0 | medium | medium | Do not publish crates or buy a domain until ADR-0001 is decided. A binary rename before 1.0 is cheap, after is expensive |
