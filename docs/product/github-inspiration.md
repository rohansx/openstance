# Open-source references and what we take from them

> Status: working document. Owner: product. Last reviewed: 2026-10-21.
> Source: a research note supplied by the maintainer (repository snapshot of 21 October 2026: READMEs, GitHub metadata and selected implementation files), checked here against the openstance code on branch `build/phase-4-ui`. Read with [competitive-analysis.md](competitive-analysis.md) and [vision.md](vision.md).

## Read this first

- **Design inspiration only.** No code from any project below has been copied into openstance, and none is planned. Where we adopt an idea, we write our own implementation against our own specs.
- **Licences decide what may even be looked at closely.** Monica and Relaticle are AGPLv3, and most of Twenty is AGPLv3 (with some commercial files and a few MIT packages). Do not copy, port or paraphrase code from them into openstance. Orbit's README says MIT, but the inspected tree had no licence file and GitHub detected none, so treat its licence as unclear. Lean Intros and Relationship Substrate are MIT; even there, reuse would need attribution and a deliberate decision, and none is planned. This is a product rule, not legal advice.
- **The projects were not run.** The research read documentation, metadata and some source files. It is a design assessment, not a security, quality or production-readiness audit.
- **Activity is a snapshot.** "Last push" and "latest commit" dates below were true on the research date. They say nothing about quality or future maintenance.

## Recommendation in one paragraph

Keep openstance's local-first direction and its scope. Take four ideas: Lean Intros' target-to-introduction workflow, Orbit's durable import identities (merge once, remember across reimports), Relationship Substrate's separation of evidence by source category, and Twenty's consistent saved views and discoverable actions. Do not fork any of them: a fork would import another project's architecture, product scope and, for three of them, a licence we cannot take.

## Shortlist

| Project | What it is | Licence (as inspected) | What we adapt | Caveat |
|---|---|---|---|---|
| [Lean Intros](https://github.com/draftboardco/lean-intros) | Self-hosted Python/SQLite warm-intro app: connection and target imports, intro statuses, optional enrichment | MIT | Target lists, choosing a connector, tracking an introduction, explaining each possible path | Closest direct alternative found; small project; last push May 2026 |
| [Orbit](https://github.com/moritzWa/orbit) | Local SQLite relationship manager with several importers and merge redirects | **Unclear**: README says MIT, no licence file, none detected by GitHub | Import review, stable source identities, a contact timeline, reimports that keep earlier merges | Small project, active October 2026. Clarify the licence before even quoting code |
| [Relationship Substrate](https://github.com/dbmcco/relationship-substrate) | Python/Postgres relationship backend with evidence references, typed source categories and identity-review tests | MIT | Keep observed interactions, imported facts, public enrichment and AI interpretation apart; keep correction history | Early backend reference, not a finished app; latest inspected commit July 2026 |
| [Twenty](https://github.com/twentyhq/twenty) | Extensible open-source CRM: objects, views, workflows, agents | Mixed: mostly AGPLv3, some commercial files, selected MIT packages (for example twenty-ui) per its [LICENSE](https://github.com/twentyhq/twenty/blob/main/LICENSE) | Consistent saved views, extensible contact fields, discoverable actions (ideas only) | Large CRM; a different product. Do not copy AGPL or commercial code |
| [Monica](https://github.com/monicahq/monica) | Personal relationship manager: notes, how-you-met context, tasks, reminders | AGPLv3 | A daily follow-up screen; context about a contact beyond a professional profile (ideas only) | `main` is labelled beta; README points to 4.x for stable; latest inspected commit September 2026 |
| [Relaticle](https://github.com/relaticle/relaticle) | CRM whose MCP server registers search, records, schema resources, tasks and email actions | AGPLv3 | Small, explicit agent tools with discoverable input schemas; separate "draft" from "send" | Active October 2026. Its workspace scoping is described in the server; authorization enforcement was not audited |

Monica already appears in [competitive-analysis.md](competitive-analysis.md); the others are added there in short form.

### The Happenstance CLI

The official [Happenstance CLI](https://github.com/happenstance-ai/happenstance-cli) is open source, but it needs Happenstance credentials and calls Happenstance's service. It is interface inspiration (how a people-search CLI presents results and asynchronous research), not a self-hosted replacement for the hosted backend. Our comparison with the service itself stays in [competitive-analysis.md](competitive-analysis.md).

## What the code showed, and what we do with it

| Finding in the reference code | Our reading | What openstance does today | What we change |
|---|---|---|---|
| Lean Intros computes work and school overlap and squashes it into a bounded score ([scoring code](https://github.com/draftboardco/lean-intros/blob/main/app.py#L6532)) | A heuristic, not a calibrated probability that two people know each other or that an intro will happen | `path` does the same kind of thing for a target outside the graph: `strength = my_score x affinity`, with fixed affinities same company 1.0, past company 0.6, school 0.4 (`crates/openstance-core/src/playbook/path.rs`). The JSON carries `reason` (`tie`, `same_company`, `past_company`, `school`) and typed evidence, including an `enrich` item with the `enrich_log` id, but no explicit "observed" vs "possible overlap" label, and both kinds of path share one `strength` field | Label every path as observed (a tie built from shared threads or meetings) or a possible overlap (an employer or school in common). Never show a possible overlap as a probability or as a known relationship ([ADR-0014](../decisions/0014-evidence-classes-and-claim-provenance.md), backlog item 2) |
| Orbit's test redirects an imported identity to the surviving contact after a merge and keeps a corrected name ([merge test](https://github.com/moritzWa/orbit/blob/main/server/src/test-merge-redirect.ts)) | Good pattern; the test simulates the operations, so it does not prove each importer behaves | Already the design: anchor identity ids, `merged_into` / `split_from` overrides, override precedence ([data-model.md](../tech-spec/data-model.md) sections 4.7 and 5). Tests ER-14, ER-15 and the property tests in `crates/openstance-core/tests/rebuild_graph.rs` and `rebuild_props.rs` cover it, but they also write rows through the `Sink` or test helpers, not through a real importer reading a second export file | Add end-to-end tests that go through `Graph::import` with a real second export (backlog item 1) |
| Relationship Substrate types its sources (direct interaction, curated export, enrichment, derived interpretation) and keeps evidence references on records ([contracts](https://github.com/dbmcco/relationship-substrate/blob/main/relationship_substrate/contracts.py)); its identity tests record a review decision without merging automatically ([identity tests](https://github.com/dbmcco/relationship-substrate/blob/main/tests/test_identity_candidates.py)) | The categories map well onto data we already store | The information exists but is spread out: `source.kind`, `interaction.kind`, `override`, `enrich:<backend>` sources, `reason_source: "model:<id>"`. Review decisions are already overrides | Name the categories once and expose them on every evidence item ([ADR-0014](../decisions/0014-evidence-classes-and-claim-provenance.md), Proposed) |
| Relaticle registers record operations next to schema resources and has separate draft-email and send-email tools ([MCP server](https://github.com/relaticle/relaticle/blob/main/app/Mcp/Servers/RelaticleServer.php)) | A small, purposeful tool set works; drafting and sending must be separate | 8 MCP tools (`crates/openstance-mcp/src/schema.rs`). openstance never sends anything; `brief` drafts text only | Add intro tools only when the intro pipeline settles; no send tool ever ([vision.md](vision.md) non-goals) |

## Gap analysis

"Today" was checked against the code on 2026-10-21. "Where planned" points at the backlog added to [../plan/roadmap.md](../plan/roadmap.md#research-backlog-2026-10-21) and [../plan/phase-5-and-beyond.md](../plan/phase-5-and-beyond.md).

| Research item | openstance today (refs) | Gap | Where planned |
|---|---|---|---|
| Reimporting the same export creates no duplicates | Same bytes: `file_hash` (blake3) match returns "already imported", writes nothing (`crates/openstance-core/src/ingest.rs`; test `reimporting_the_fixtures_is_a_no_op` in `tests/fixtures_expected.rs`). A *new* export of overlapping data: identities are `UNIQUE(kind, value)` so no duplicate people; email and meeting interactions are deduped across sources ([ingestion.md](../tech-spec/ingestion.md) section 5, layer 3) | Connection and contact rows are deliberately kept per export (the `watch` diff needs them). From reading `score/load.rs`, a second LinkedIn export adds a second `connection` row per person, which counts once more in F. Not covered by a test | P4-IMP-02 (test and decide) |
| Import preview of duplicates and merges | `import --dry-run` is specified in [interfaces.md](../tech-spec/interfaces.md) and [ingestion.md](../tech-spec/ingestion.md) section 9 | **Not implemented**: `ImportArgs` in `crates/openstance/src/lib.rs` has no `--dry-run`; `--replace` returns "not supported yet". No preview of which people would merge | P4-IMP-01 |
| Reviewed merges survive later imports | Confirm/merge/split are overrides; ER-14 (split holds after an exact-match import), ER-15 (merge survives rebuild), property tests (incremental equals full) | Tests write through the `Sink` or helpers, not through `Graph::import` with a real second export file of each importer | P4-IMP-02 |
| Name-only similarity requires review | Rules 5 and 6 (name only) never merge; Jaro-Winkler only suggests ([entity-resolution.md](../tech-spec/entity-resolution.md) section 3) | None for name only. Note: name + company (0.85) and name + shared thread (0.75) **do** auto-merge, flagged in `review` and reversible. That is stricter than "suggest only" would be for recall, looser for safety; it is a deliberate choice in [ADR-0009](../decisions/0009-conservative-entity-resolution.md) | Measured in the pilot (mistaken-merge metric, below) |
| Evidence drawer on every suggested connection | Evidence lines on results (date, kind, direction, subject, source label); tie evidence on `how_connected`; `Evidence` variants `interaction`, `field`, `tie`, `enrich` (`crates/openstance-core/src/json.rs`). UI person page shows mutuals and a "how connected" panel | No single drawer that shows, per hop, source, date, whose relationship it is, the class of evidence, and what is missing | P4X-UI-01 |
| Distinguish observed relationships from possible overlaps | `path` returns `reason` per path; tie paths and employer/school paths both report `strength` | No explicit label; employer/school intersections from enrichment read like warm paths | P4-PATH-01, [ADR-0014](../decisions/0014-evidence-classes-and-claim-provenance.md) |
| Per-claim source category | Implicit in `source.kind`, `interaction.kind`, `override`, `enrich:<backend>`, `reason_source` | Not named, not on the JSON | [ADR-0014](../decisions/0014-evidence-classes-and-claim-provenance.md) (Proposed), P4X-CORE-01 |
| "If no path, say so" | CLI `how` prints "no warm paths to X: nobody you know shares a thread or meeting with them"; CLI `path` prints "no warm paths found" (plus a note when egress is needed) | `how_connected` JSON has no reason field (`target`, `direct`, `paths` only); `path` sometimes adds a free-text note (for example "target not in graph; enable an enricher to look up employers"). The UI and agents mostly infer "no path" from an empty array | P4-PATH-02 |
| Targets: person or company goal, inspect connectors, prepare a request | `path` and `accounts` playbooks; shortlist; intro rows with an optional connector; `brief` drafts text | No saved "target" object; the intro row is the closest thing | Covered by the intro pipeline (P4X-INTRO-*); a separate target table is not planned |
| Intro pipeline: draft, requested, accepted, introduced, declined, closed; next action; outcome | `intro` table, stages `draft, asked, intro_made, met, closed`, note, `follow_up` date, optional connector (migration 0004, [ADR-0013](../decisions/0013-react-vite-web-ui-and-workspace.md)). HTTP and UI only | No `accepted` or `declined`; no next-action field; no CLI or MCP surface; no "due" list | P4X-INTRO-01..03 |
| Follow-ups and commitments (promise, due, snooze, done; AI suggestions only) | Override `note` rows; intro `follow_up` date; `reconnect` playbook | Nothing for promises | P5-MEM-01..03 |
| Agent tools `search_people`, `get_evidence`, `list_intro_paths`, `create_intro_draft` | `search_people` exists. `get_person` returns evidence; `how_connected` / `path_to` return paths with evidence; `brief` drafts. ADR-0011 already rejected a duplicate `who_knows` tool | Evidence and path tools are covered by existing tools under other names. No intro tools | P4X-MCP-01 (intro tools only) |
| Reads respect excluded contacts and sharing settings | `do_not_share` excludes people from views; `no_enrich` from egress; bulk hidden from search | No new gap found for local reads; federation leakage is already a Phase 5 risk | Phase 5 WP3 (existing) |
| Provider adapters with provenance, timestamps, coverage, cost; keys on the backend; partial failures recorded; export | `Enricher` trait, quotes in micro-USD, `enrich_log` with request, status and cost, `partial` status, keys from env/config, `enrich:<backend>` sources (`crates/openstance-enrich`, [enrichment.md](../tech-spec/enrichment.md)) | Coverage (how many fields came back) is not summarised; no people-discovery op (only profile, find/verify email, company, web search) | P5-RES-01..03 |
| Exa as public-web research | Integrations page lists Exa as *Planned*, "Not built. Public discovery is not evidence of a warm relationship" (`ui/src/pages/Integrations.tsx`). treg's catalog lists `exa.people.search` and `exa.web.search` (checked 2026-10-21) | No people-search op in `Enricher`; treg backend maps `web_search` to `treg.web.search`, not to Exa specifically | P5-RES-02 |
| Boardy stays a labelled handoff | Integrations card: method "Email handoff", status *Planned*, "Not built. No public API or webhook documentation was found in our review" | None today; keep it that way until an authorised integration is verified | P5-RES-03 |

## Positioning to test

> Find the right person, understand why the connection matters, and follow through—with a network you control.

This is a line to test with users, not launch copy. The launch positioning in [../plan/launch.md](../plan/launch.md) stays as it is until the pilot says otherwise.

### Three validation tasks

1. **Import without cleanup surprises.** Import two or three of your own exports, then reimport a newer one.
2. **Find an evidence-supported introduction path** to someone you actually want to meet, and judge whether the evidence is right.
3. **Complete the follow-up**: ask the connector, record the outcome, and close the loop.

### What we measure

| Metric | How (no telemetry; see Q11 in [../risks-and-open-questions.md](../risks-and-open-questions.md)) |
|---|---|
| Time to the first useful result | Pilot sessions, stopwatch from first import to a result the participant would act on |
| Mistaken identity merges | Participant reviews flagged merges and a sample of automatic ones; count splits and "wrong person" reports |
| Whether the connector confirms the suggested relationship | Ask the participant to ask the connector; record yes, no or no answer per suggested path, split by observed vs possible overlap |
| Completed introductions | Intro rows that reach `introduced` (today `intro_made`) or `met`, reported by the participant |

More integrations or a larger graph are not evidence of a better product. If these four numbers do not move, adding sources will not fix it. This matches the first validation question in [vision.md](vision.md#success-measures).

## Related documents

- [competitive-analysis.md](competitive-analysis.md), [vision.md](vision.md), [personas-and-use-cases.md](personas-and-use-cases.md)
- [../decisions/0014-evidence-classes-and-claim-provenance.md](../decisions/0014-evidence-classes-and-claim-provenance.md)
- [../plan/roadmap.md](../plan/roadmap.md#research-backlog-2026-10-21), [../plan/phase-5-and-beyond.md](../plan/phase-5-and-beyond.md)
- [../risks-and-open-questions.md](../risks-and-open-questions.md) (R27 to R30, Q25 to Q29)
