# Phase 2 (Week 2): semantic search, agents, and the public MVP

**Timebox:** 5 working days. **Release:** `v0.3.0` + **Show HN**. **Previous:** [Phase 1](phase-1.md). **Next:** [Phase 3](phase-3.md). **Overview:** [roadmap](roadmap.md). **Launch plan:** [launch.md](launch.md).

## Goal

Search moves from keyword to meaning. The same graph is available to agents. Then it ships.

When this phase ends, someone can import three exports, ask "fintech founders I've actually met" and get ranked results with evidence lines, ask "how am I connected to Dana Whitfield?" and get the top three paths with the threads behind them, and do all of that from inside Claude Code through `openstance mcp`. It works offline, with no API key, at no cost. That is the public MVP, and it is what the Show HN post demonstrates.

### Relation to the overview's MVP

The product overview's two-week MVP also included `path` to a stranger through treg and `share` to an encrypted view file. In the reconciled plan those features move to [Phase 3](phase-3.md), one week after launch. The launch demo uses `how_connected` (a target that is already in your graph), which needs no egress and is still free. Week 3 then ships the paid-egress path as a follow-up announcement. This keeps the launch promise ("free, offline, nothing leaves your machine") clean on day one.

## Scope

In scope:

- `openstance-model`: the fastembed `Embedder` (bge-small-en-v1.5, 384 dimensions, quantized, downloaded once and cached).
- `person_vec` (sqlite-vec `vec0`) through migration `0002`, plus incremental embedding.
- Hybrid fusion of FTS5 and vector results with the score multiplier.
- `who` (one person with score components, evidence, and top mutuals).
- `how_connected` (top 3 paths with evidence).
- `openstance mcp`: a stdio MCP server built on `rmcp` with `search_people`, `get_person`, and `how_connected`.
- A first agent skill in `playbooks/`.
- Search eval expanded to 50 queries.
- The 60-second demo recording, the README, and Show HN.

Out of scope: enrichment and treg, `path`, `brief`, views, HTTP, MCP over HTTP, and LLM parser or reranker. See the [roadmap](roadmap.md#what-is-out-of-scope-per-phase).

---

## Task breakdown

### `openstance-model`

- [ ] **P2-MODEL-01 fastembed Embedder.** `FastEmbedder` wraps `fastembed` with `BGESmallENV15Q` (or the quantized variant available at implementation time), loads the model from `~/.openstance/models/`, and downloads it on first use (about 33 MB) with a progress bar. Add an `embedder` config key such as `fastembed:bge-small-en-v1.5`.
  - *Acceptance:* `dims() == 384`. Embedding the same text twice gives identical vectors. If the model is missing and the network is unavailable, the error says "run once with network or set `model.embedder = \"none\"`", and search falls back to FTS only instead of failing.
- [ ] **P2-MODEL-02 Embedder selection from config.** A factory maps the config string to `Box<dyn Embedder>`, and `none` maps to `NullEmbedder`. The active embedder id and dims are stored in a `meta` key (or config) so that a change can be detected.
  - *Acceptance:* changing `model.embedder` and running any command prints "embedder changed, run `openstance rebuild --embeddings`". Search uses FTS only until that has been done.
- [ ] **P2-MODEL-03 Throughput.** Batch embedding (batch size tuned, default 64) uses all cores through the fastembed thread pool.
  - *Acceptance:* the first-run embedding of 20k people finishes in under 90 s on an M2 CPU (perf job, macOS runner or manual sign-off).

### `openstance-core`

- [ ] **P2-CORE-01 sqlite-vec integration + migration `0002_person_vec.sql`.** Statically link and register sqlite-vec on every connection. Create `person_vec USING vec0(person_id INTEGER PRIMARY KEY, embedding float[384])`.
  - *Acceptance:* the migration applies to a v0.2.0 database without data loss. A smoke test runs `vec_distance_cosine`. Using an embedder with different dims recreates the table only through `rebuild --embeddings`.
- [ ] **P2-CORE-02 Embedding text and incremental index.** Embedding input = `display_name + title + company + bio + top subjects` (capped at 512 tokens). Store an input hash per person (a column or side table added in the same migration) and re-embed only people whose hash changed after import, edit, merge, or rebuild.
  - *Acceptance:* editing one title re-embeds exactly one person. `rebuild` with no source changes re-embeds no one.
- [ ] **P2-CORE-03 Fusion.** Candidates = union of the top 200 FTS (bm25) hits and the top 200 vector (cosine) hits, then filtered by the structured fields of `Query`, with bulk and merged people excluded. bm25 and cosine are each min-max normalised over the candidate set. `text_score = 0.5 x bm25_n + 0.5 x cos_n` (a missing signal counts as 0). `final = text_score x (0.5 + 0.5 x person.score)`. See [search](../tech-spec/search.md).
  - *Acceptance:* unit tests on hand-built candidate sets check that the normalisation handles a single candidate (no division by zero) and that a strong relationship lifts a weaker text match as intended. Results expose `fts_rank`, `vec_sim`, and `final` in the JSON.
- [ ] **P2-CORE-04 `who`.** `get_person(id|email|name)` returns the full record (fields with their provenance: derived source vs override), identities with confidence, all five score components and the weights used, the last 10 interactions, and the top 5 mutuals by tie weight.
  - *Acceptance:* finishes in under 20 ms on the perf fixture. A person with `do_not_share` is returned locally (that flag only affects sharing), and the flag is shown.
- [ ] **P2-CORE-05 `how_connected`.** Given a target in the graph, consider each mutual `m` with ties to the target: `strength = my_score(m) x tie_weight(m, target)`. Return the top 3 paths. Each path carries its evidence: the most recent shared interactions between `m` and the target (joined through `interaction_person`, never stored), plus my own last interaction with `m`. If I have a direct relationship with the target, return it as path 0.
  - *Acceptance:* finishes in under 20 ms on the perf fixture. The fixture test gives the expected top mutual. If the target is not in the graph, the result is `{ "found": false, "hint": "path_to (Phase 3) can look outside your graph" }` and no egress happens.
- [ ] **P2-CORE-06 Search eval tier 1 to 50 queries.** Extend `evals/search.toml` to 50 queries that mix structured, free-text, and semantic phrasing ("people who build developer tools" should match "devtools", "SDK", and "API platform" titles).
  - *Acceptance:* CI runs it with `parser = rules`, `reranker = none`, and the fastembed embedder (the model is cached between CI runs). Recall@5 is reported and gated. A second tier that runs Ollama is documented for local use only (see [testing strategy](../engineering/testing-strategy.md)).
- [ ] **P2-CORE-07 Latency gate.** Add a criterion bench for text search with the embedder.
  - *Acceptance:* p50 under 300 ms end to end (including query embedding) and structured search under 50 ms, on the perf fixture. Both are tracked in CI as benchmarks, and a regression of more than 20 % fails.

### `openstance-mcp`

- [ ] **P2-MCP-01 rmcp stdio server.** The `openstance-mcp` crate depends on tokio and rmcp. `openstance mcp` starts it on stdio and opens the database read-mostly (WAL, so the CLI can import at the same time).
  - *Acceptance:* `npx @modelcontextprotocol/inspector openstance mcp` lists the tools and calls each one successfully against the fixtures.
- [ ] **P2-MCP-02 Tool `search_people`.** Input: `{ text?, company?[], title?[], location?[], industry?[], tags?[], since?, until?, last_contact_before?, met_in_person?, replied?, min_score?, lost_touch?, include_bulk?, limit?, parse?, explain? }` (exact schema in [interfaces](../tech-spec/interfaces.md#22-input-schemas)). Output: the same JSON as `openstance search --json`, as MCP structured content plus a short text summary. It also returns the parsed `Query`, so the agent can show the filters and rerun them. Company filtering plus `how_connected` covers the overview's "who knows someone at X". There is no separate tool for it.
  - *Acceptance:* a JSON schema snapshot test. The output for a fixed query is byte-identical to the CLI `--json`.
- [ ] **P2-MCP-03 Tool `get_person`.** Input: `{ person, interactions?, mutuals? }`, where `person` is an id, email, LinkedIn URL/slug, `@x_handle` or name. Output: the `who` JSON (`PersonDetail`).
  - *Acceptance:* an ambiguous name returns the `ambiguous` error with `candidates` ([interfaces](../tech-spec/interfaces.md#111-error)) rather than a guess.
- [ ] **P2-MCP-04 Tool `how_connected`.** Input: `{ target, limit? }` (id, email, LinkedIn, or name). Output: the top 3 paths with evidence.
  - *Acceptance:* the same snapshot rule as P2-MCP-02.
- [ ] **P2-MCP-05 Agent skill `playbooks/openstance-search.md`.** Written in our own words. It says when to call `search_people` vs `get_person` vs `how_connected`, how to show the evidence lines to the user instead of paraphrasing them, and that the tools cost nothing and send nothing anywhere. It contains no third-party promotion.
  - *Acceptance:* reviewed by a second person. In Claude Code, the demo questions trigger the right tools without extra prompting.
- [ ] **P2-MCP-06 Client setup docs.** Snippets for Claude Code (`claude mcp add openstance -- openstance mcp`), Cursor (`mcp.json`), and Claude Desktop config.
  - *Acceptance:* each snippet is tested by hand once and recorded in the PR.

### `openstance` (CLI)

- [ ] **P2-CLI-01 `who <name|email|id>`.** A human view (the score as a bar per component with the weights, then identities, the last interactions, and mutuals), plus `--json`.
  - *Acceptance:* the output answers "why 0.81" with no other command needed.
- [ ] **P2-CLI-02 `how "<target>"`** (alias `how-connected`). Prints the top 3 paths: `you -> Mutual (score 0.74) -> Target (tie 0.62)`, followed by evidence lines (`2026-03-12 email "devtools dinner follow-up" (cc'd together)`).
  - *Acceptance:* `--json` is identical to the MCP output. The command name matches [interfaces](../tech-spec/interfaces.md).
- [ ] **P2-CLI-03 `rebuild --embeddings`** and showing embedding progress during `import`.
- [ ] **P2-CLI-04 `mcp`.** Starts the stdio server and logs to stderr only (stdout is the protocol).
  - *Acceptance:* a test checks that nothing but JSON-RPC appears on stdout.

### Demo, docs, launch (`docs`)

- [ ] **P2-DOC-01 Demo dataset `fixtures/demo/`.** A synthetic but realistic network (about 600 people, 3 exports: `takeout.mbox`, `Connections.csv`, `contacts.vcf`) built so that the demo queries give clean, believable answers. All people and companies are fictional and labelled that way in the README.
- [ ] **P2-DOC-02 Record the 60-second demo** (script below) as an asciinema cast plus a GIF/MP4 for the README.
  - *Acceptance:* it can be reproduced from a clean machine with network access blocked after the model download.
- [ ] **P2-DOC-03 README for launch.** Meets every requirement in [launch.md](launch.md#readme-requirements).
- [ ] **P2-DOC-04 Show HN post and first comment drafted**, following [launch.md](launch.md#channels).
- [ ] **P2-CI-01 Tagged `v0.3.0` source release** with `cargo install --git` instructions. Prebuilt binaries can be added early if cargo-dist is quick to set up. Otherwise they arrive in Phase 4.

### Parallelisation

Three streams: (1) embedder plus vec plus fusion (MODEL-01..03, CORE-01..03, CORE-07), (2) graph queries (CORE-04/05, CLI-01/02), (3) MCP (MCP-01..06). The MCP stream can start on day 1 against the Phase 1 search and pick up fusion when it lands. Demo and docs work starts on day 3.

---

## Deliverables

- `v0.3.0` tag. Commands added: `who`, `how` (alias `how-connected`), `mcp`, and `rebuild --embeddings`.
- MCP tools: `search_people`, `get_person`, `how_connected`.
- `playbooks/openstance-search.md` agent skill.
- 50-query search eval in CI.
- The 60-second demo (asciinema + GIF/MP4) and the launch README.
- The Show HN post.

## The 60-second demo script

This recording goes at the top of the README and in the Show HN post. It runs on `fixtures/demo/`. No API key. Wi-Fi is off after the first model download, and the recording should show the Wi-Fi icon off. Target length is 60 seconds at normal typing speed (use `asciinema rec --idle-time-limit 1`).

| Time | Screen | Command / action | What the viewer should notice |
|---|---|---|---|
| 0:00-0:05 | Terminal, Wi-Fi off | `ls ~/exports` shows `takeout.mbox Connections.csv contacts.vcf` | These are files you can download yourself today |
| 0:05-0:15 | Terminal | `ost import ~/exports/*` | Progress bar, then `612 people, 41,880 interactions, 3 merges to review` in seconds |
| 0:15-0:27 | Terminal | `ost search "fintech founders I've actually met"` | The parsed filters line (`title=founder industry=fintech met_in_person=true`), then 5 results, each with a score and an evidence line such as `2026-03-12 meeting "Seed round coffee"` |
| 0:27-0:35 | Terminal | `ost who "Maya Chen"` | Score 0.81 broken down: recency 0.92, frequency 0.55, reciprocity 0.88, meetings 0.74, breadth 0.50. Shows that this is a number you can argue with |
| 0:35-0:45 | Terminal | `ost how-connected "Dana Whitfield"` | Three paths with mutuals and the threads behind each tie |
| 0:45-0:57 | Claude Code | Prompt: *"Who do I know at Lumen Pay, and who's my warmest way in to their CTO?"* | Claude calls `search_people` (company filter) and then `how_connected`, and answers with evidence lines it did not make up |
| 0:57-1:00 | Title card | "openstance: your network as a file. Free search, works in Claude Code. MIT." | Repo URL |

Pre-flight before recording:

```sh
rm -rf ~/.openstance && ost init --me you@example.com
ost import fixtures/demo/*            # warm the model cache, then:
rm -rf ~/.openstance/graph.db* && ost init --me you@example.com
claude mcp add openstance -- ost mcp
# turn Wi-Fi off, start asciinema
```

## Exit criteria

1. All P2 tasks are closed or moved with a link.
2. Search eval: recall@5 meets or beats the threshold recorded in `evals/search.toml`, with the embedder on. Turning the embedder off never drops recall below the Phase 1 FTS-only baseline.
3. Latency on the perf fixture: text search p50 under 300 ms, structured under 50 ms, `who` and `how_connected` under 20 ms, first-run embedding of 20k people under 90 s.
4. MCP: the inspector session succeeds, and the Claude Code demo question works in 3 out of 3 fresh sessions.
5. The demo recording plays back from a clean install with no network after the model is cached.
6. Gate A in the [roadmap](roadmap.md#gate-a-launch-readiness-end-of-phase-2) passes, and the Show HN post is live.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| fastembed / ONNX runtime linking problems on some platforms | medium | high | Use the `ort` download strategy that fastembed recommends. Test on macOS arm64 and Linux x86_64 in CI. With `embedder = none` the binary still works |
| sqlite-vec static linking with bundled rusqlite | medium | medium | Prototype it on day 1. The fallback is brute-force cosine in Rust over a BLOB column, which is fine at 20k vectors |
| Semantic hits that look plausible but are wrong in the demo | medium | high | Evidence lines on every result. The score multiplier pushes down cold contacts. Hand-check the demo queries |
| MCP client differences (Claude Code vs Cursor) | medium | medium | Test both. Keep the tool schemas flat and simple |
| Launch timing slips because of polish | high | medium | The demo and README are the launch. A UI is not required. Hold to Gate A and nothing more |
| Show HN post reads as "a better Happenstance" and draws a dismissive or legal response | low | high | Lead with "free search, your graph as a file, agent-native", use factual comparison only, and no parody (see [launch.md](launch.md)) |
