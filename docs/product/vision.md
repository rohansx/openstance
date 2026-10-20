# Product vision

> Status: accepted for v1 planning. Owner: product. Last reviewed: 2026-10-20.
> Sources: *openstance product overview* (PDF, 2026-10-20), *openstance tech spec* (PDF, 2026-10-20), and the sourced research in the v0.1 prototype (`dist/research.md`, summarized in [../prototype-v0.1.md](../prototype-v0.1.md)). Where the research corrects the overview, this document follows the research.

## What openstance is

openstance is an open-source, local-first people-search engine over **your own network**. Your relationships, imported from the exports you already have (Gmail Takeout, LinkedIn `Connections.csv`, contacts `.vcf`, calendar `.ics`, the X archive and later WhatsApp, Slack and notes), become **one SQLite file on your machine**. You search that file in plain English or with structured flags from a CLI, from any MCP-capable agent, or from a small local web UI. Search is unlimited and free. Outside data (profile lookups, email find/verify, web search) is pulled in only when you ask, per call, with the price shown before anything leaves the machine.

**One line:** your network as a file. Search it with any model. Share slices, never the graph.

**Working name.** The product, binary and crates are called `openstance` (short alias `ost`). The overview recommends renaming to "kith" because of trademark risk around "stance" and to avoid reading as a parody. That is an open decision tracked in [ADR-0001](../decisions/0001-product-name.md); until it is resolved, every doc, crate and command uses `openstance`.

| Fact | Value |
|---|---|
| Binary | `openstance`, alias `ost` |
| Data dir | `~/.openstance/` (graph at `~/.openstance/graph.db`, config at `~/.openstance/config.toml`) |
| Shared view files | `*.ost` (age-encrypted SQLite) |
| Local HTTP API / UI | `127.0.0.1:7411` |
| Language / store | Rust workspace; SQLite + FTS5 + sqlite-vec |
| License | MIT (prototype); see [../decisions/README.md](../decisions/README.md) for the core license decision |

## The problem

People who depend on warm introductions (founders raising, job seekers, recruiters, sellers, community managers) already have the data: years of email headers, calendar invites, LinkedIn connections, DMs and meeting notes. What they lack is a way to ask a simple question of it ("who do I know at Stripe who actually replies to me?") without one of two bad options:

1. **Upload the whole graph to a hosted service.** This works well (Happenstance is the strongest example; see [competitive-analysis.md](competitive-analysis.md)), but the relationship graph is among the most sensitive datasets a founder owns, and on hosted products search beyond a free allowance, export and API/MCP access are typically metered or paid.
2. **A spreadsheet or a personal CRM.** Ownership is fine, but nobody keeps it current, it has no relationship signal, and it cannot answer "how am I connected to this stranger?".

The evidence behind each pain point is uneven. The research for v0.1 graded them; we keep those grades honest:

| Pain | Evidence strength | openstance response |
|---|---|---|
| A usage limit or upgrade prompt interrupts a search workflow | Reported by the founding user; chat limits confirmed on Happenstance's pricing page | No application-imposed limit on local search |
| Exporting results requires a paid tier | Confirmed on Happenstance's pricing page (CSV export is a Pro feature) | The SQLite file is the format; CSV/JSON/vcf export is always free |
| A plausible result does not actually meet the criteria | One public review reports a misclassified role; not a representative study | Show the extracted filters, matched fields and evidence lines; let the user edit and rerun without a model |
| A connection does not mean someone will make an intro | Hypothesis, to be tested | Separate fit (search) from reachability (score, paths with evidence) |
| Uploading one's network creates an ownership concern | Structural tradeoff, not evidence of misuse | Local file, explicit per-view sharing, no account required |
| Sparse or stale records make discovery weak | Expected constraint, prevalence unmeasured | Unknown fields stay unknown; overrides; opt-in, quoted refresh |

## Thesis: three bets

### Bet 1. Search should be free

Most of the work in answering "who do I know who..." is deterministic: parsing exports, resolving identities, computing a relationship score, filtering by company/title/date, full-text and small-vector retrieval. All of that runs locally on a laptop in milliseconds for $0 (targets in [../engineering/performance-targets.md](../engineering/performance-targets.md)). Hosted products that run an LLM agent loop on every query have a real marginal cost per search and therefore meter it. openstance moves the expensive step out of the default path: a model is optional, used only to parse a query into filters and to rerank or explain the tail, and the pipeline degrades to pure SQL when no model is configured (see [../tech-spec/search.md](../tech-spec/search.md)).

*What would prove it wrong:* if useful answers on real users' data require an LLM with web search on most queries, local-first search is a worse product, not a cheaper one. The Phase 2 search eval (50 queries, rules parser, no reranker) is the test.

### Bet 2. The graph is the product, so the user should own it

The relationship graph is the asset. openstance stores it as a plain SQLite file with a documented schema ([../tech-spec/data-model.md](../tech-spec/data-model.md)): raw source rows are kept, everything derived (people, ties, scores, indexes) can be rebuilt, and user corrections live in their own `override` table so neither re-import nor enrichment ever overwrites an edit. Backup is `cp`. Export is one command. Sharing is a named, encrypted, read-only slice (`*.ost`), never the graph.

*What would prove it wrong:* if target users will not run a local binary or do not care where the graph lives once search is good, ownership is a feature, not a reason to switch.

### Bet 3. Agents are the front door

Claude Code, Cursor, ChatGPT connectors and similar agents will query people graphs more often than humans type into a search box. openstance ships an MCP server in the same binary (`openstance mcp`) with eight tools (`search_people`, `get_person`, `how_connected`, `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich`), free, returning the same JSON as the CLI and HTTP API. Having MCP is not a differentiator on its own (Happenstance has MCP too); having it **local, free, and with explicit egress control on every money-spending tool** is.

*What would prove it wrong:* if Show HN and agent-directory traffic is flat, the agent channel is not the distribution channel we assumed. See the kill criterion below.

## Principles

These are the rules that decide design arguments. Each one has a concrete consequence in the spec.

1. **Local-first.** The graph lives in `~/.openstance/graph.db`. The binary works offline with zero API keys. There is no openstance server in the default loop. Email ingestion is headers-only by default (`import.email_bodies = false`); bodies are opt-in. The HTTP API binds to `127.0.0.1:7411` behind a bearer token.
2. **Explainable.** Every result carries evidence lines that are facts, not prose: thread subjects, meeting titles, who cc'd whom, dates. The relationship score is a weighted sum of five visible components (recency R, frequency F, reciprocity C, meetings M, breadth B) with weights in TOML; see [../tech-spec/scoring.md](../tech-spec/scoring.md). Extracted filters are shown and editable. No fabricated "97% match" badges. A model inference is never presented as an imported fact.
3. **Export-first.** Ingestion starts from exports people can download today, not OAuth connectors that need app verification. Output is always exportable (`openstance export --format csv|json|vcf|parquet`). No paid unlock for data portability, ever, including in a future cloud tier.
4. **Agent-first.** One JSON shape across CLI (`--json`), HTTP and MCP ([../tech-spec/interfaces.md](../tech-spec/interfaces.md)). Playbooks ship as markdown skill files that tell an agent which tool to call; the skill text is openstance's own and contains no third-party promotion.
5. **Free search, paid egress only.** Search, import, scoring, MCP, CLI, API and playbooks are free. Money is spent only when data has to come from outside, and then: enrichment backend is `none` by default, `allow_egress` defaults to `false`, every egress is quoted first, a per-run budget (`enrich.max_usd_per_run = 0.50`) aborts above the cap, every call is logged to `enrich_log`, and per-person `do_not_share` and `no_enrich` flags are always respected. See [../tech-spec/enrichment.md](../tech-spec/enrichment.md).
6. **Conservative by default.** A wrong merge is worse than a missed one. Auto-merge only on strong joins; fuzzy matches are suggestions surfaced in `openstance review`. Bulk senders (newsletters, notifications) are flagged `is_bulk`, scored 0 and excluded from search by default.
7. **Pluggable, not vendored.** Importers, enrichers, embedders, parsers and rerankers are traits with a Null implementation. treg is one `Enricher` implementation among `none` and own-keys; core never imports a concrete provider.

## What v1 is

v1 is the end of Phase 4 in the [roadmap](../plan/roadmap.md): a single binary, installable with one command, that does the following:

- imports mbox (Takeout), LinkedIn CSV, vcf, ics and the X archive;
- resolves people conservatively with a review queue, merge and split;
- scores relationships with presets (founder, recruiter, seller, community);
- searches with rules or a model parser, FTS5 and local embeddings, with evidence;
- answers `who`, `how_connected` and runs the `path`, `brief`, `accounts`, `refresh`, `watch`, `reconnect` and `verify` playbooks;
- shares named views as age-encrypted `.ost` files;
- exposes everything over CLI, MCP and a localhost HTTP API with a minimal embedded UI.

The **public MVP** is the end of Phase 2 ([../plan/phase-2.md](../plan/phase-2.md)): three exports imported, a free search with evidence lines, `how_connected`, and the same from Claude Code via `openstance mcp`, recorded as a 60-second terminal demo and launched as a Show HN ([../plan/launch.md](../plan/launch.md)).

## Non-goals for v1

| Non-goal | Why | Revisit |
|---|---|---|
| iOS or any mobile app | A mobile client needs sync and a hosted endpoint; both are cloud-tier work | After cloud decision ([pricing-and-cloud.md](pricing-and-cloud.md)) |
| Groups / pooled community networks | Happenstance's moat is group reach (500k+ users, large groups). A local tool cannot replicate network effects in v1 and should not try. Federated views over git/S3 come in Phase 5 | [../plan/phase-5-and-beyond.md](../plan/phase-5-and-beyond.md) |
| Breadth of live OAuth connectors | Google/Microsoft app verification is slow and costly; exports deliver value in five minutes without it. IMAP with app passwords is the closest v1 gets | Phase 5, optional; managed OAuth is a cloud feature |
| A public people index or data operation | A different business (The Swarm, data vendors). Outside data comes only via opt-in enrichers | Not planned |
| Sending email or intros on the user's behalf | Brief and draft only; the user sends | Not planned for v1 |
| Open-ended agent loops | Playbooks are fixed step lists; the only branch is "enough local data, skip egress" | Not planned |
| Feature parity with Happenstance | We compete on ownership, cost and explainability, not breadth | n/a |
| A columnar/analytics store | SQLite covers the 20k-person / 500k-interaction graph; Postgres or DuckDB only for a large cloud team tier | [../decisions/README.md](../decisions/README.md) |

## Success measures

The product is judged on outcomes, not features. These are gates, not achieved results.

| Measure | Target | Where measured |
|---|---|---|
| Time to first useful shortlist after install | under 5 minutes, zero API keys | pilot sessions, demo |
| Top-5 relevance on queries answerable from imported data | at least 80% judged relevant | search eval set ([../engineering/testing-strategy.md](../engineering/testing-strategy.md)) |
| Unsupported facts in evidence lines | zero | eval review, bug reports |
| Data loss on import/export/rebuild | zero; rebuild is idempotent | property tests |
| Structured search latency | under 50 ms (text + embedder under 300 ms) | [../engineering/performance-targets.md](../engineering/performance-targets.md) |
| Unquoted egress events | zero; every egress row in `enrich_log` has a prior quote | playbook tests with `MockEnricher` |
| Launch signal | meaningful Show HN response and organic MCP/agent installs | [../plan/launch.md](../plan/launch.md) |

**First validation question** (from the research): can a founder use openstance to find one relevant person they had forgotten, explain why that person fits, and reach an accepted introduction faster than with their current process, while keeping control of their data? If that loop works repeatedly, expand. If it does not, more integrations will not fix it.

**Kill / park criterion:** if there is no meaningful Show HN response within a month of launch, park the project. Scope is deliberately one binary and one demo so that parking is cheap. Tracked in [../risks-and-open-questions.md](../risks-and-open-questions.md).

## Related documents

- [personas-and-use-cases.md](personas-and-use-cases.md): who uses it and how
- [competitive-analysis.md](competitive-analysis.md): Happenstance and adjacent tools, factually
- [pricing-and-cloud.md](pricing-and-cloud.md): what stays free, what a cloud tier would sell
- [../architecture/overview.md](../architecture/overview.md): how the binary is built
- [../plan/roadmap.md](../plan/roadmap.md): phases 0 to 5
- [../glossary.md](../glossary.md): terms used across the docs
