# Phase 3 (Week 3): opt-in egress, playbooks, and sharing

**Timebox:** 5 working days. **Release:** `v0.4.0`. **Previous:** [Phase 2](phase-2.md). **Next:** [Phase 4](phase-4.md). **Overview:** [roadmap](roadmap.md).

## Goal

openstance can now reach outside the graph, but only when asked, only after showing the price, and with an audit trail. Users can also share a slice of the graph without sharing the graph.

When this phase ends, `openstance path "Dana Whitfield, CTO at Lumen Pay"` works for someone who is *not* in your graph. It looks up their employers and school through an enricher, intersects them with your graph, ranks the mutuals, and can optionally find and verify an email. It prints a quote first and stops unless egress is allowed. `openstance brief` writes the 10-line intro brief. `openstance view export` writes an age-encrypted `.ost` file that a colleague can import and search. Calendar `.ics` and X archive imports work.

This phase is the follow-up announcement one week after Show HN: "the paid part is cents per call, shown first, and off by default".

## Scope

In scope:

- `openstance-enrich`: the `Enricher` trait, `NullEnricher`, `TregEnricher`, and a `MockEnricher` for tests. The `OwnKeys` enricher is only a compiled-out stub.
- Core egress policy: quote first, budget, `no_enrich`, `allow_egress`, and `enrich_log`. See [enrichment](../tech-spec/enrichment.md).
- The `Playbook` trait (`plan`/`run`) and the playbooks `path` and `brief`. See [playbooks](../tech-spec/playbooks.md).
- Views: `view create`, `view export`, `view import`, `view list`, `view search`, plus the age keypair. See [sharing and views](../tech-spec/sharing-and-views.md).
- Importers: `ics` and `x_archive`.
- MCP tools: `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich`.
- `export --format vcf`.

Out of scope: HTTP API and UI, the `watch` / `reconnect` / `accounts` / `refresh` / `verify` playbooks, LLM adapters, `openstance sync`, and OwnKeys providers. See the [roadmap](roadmap.md#what-is-out-of-scope-per-phase).

### Privacy defaults this phase must not break

Read these before writing any egress code. Any change to them needs an ADR in [decisions](../decisions/README.md).

- `enrich.backend = "none"` by default. A fresh install makes no egress calls, ever.
- `enrich.allow_egress_default = false`. Every money-spending entry point (CLI, MCP, and later the API) takes `allow_egress` (default false) and `max_usd`. When egress is not allowed, it returns the quote and does nothing else.
- `enrich.max_usd_per_run = 0.50` is a hard ceiling enforced in core. A per-call `max_usd` can lower it, never raise it. Raising it means editing the config.
- `no_enrich` people are never sent to an enricher. `do_not_share` people never appear in a view.
- Every egress request writes an `enrich_log` row (what was sent, to which backend, the quoted cost, the actual cost, and the outcome) before the result is used.
- treg is one `Enricher` implementation behind the trait. We do not vendor treg's skill file, and our playbook skills do not promote any third party.

---

## Task breakdown

### `openstance-enrich`

- [x] **P3-ENR-01 Trait and types.** Define `Enricher { name, quote, enrich }`, `EnrichRequest::{Profile{name, company, linkedin}, FindEmail{name, domain}, VerifyEmail{email}, Company{domain}, WebSearch{query}}`, `Quote { usd_micros, calls, breakdown }`, and `EnrichResult` (typed profile, email, company, and web hits, each with provider provenance).
  - *Acceptance:* the types serialise to the JSON documented in [interfaces](../tech-spec/interfaces.md). The trait is object-safe and sync. Backends that need async run their own small runtime internally, or use blocking HTTP, so that core stays sync.
  - *Status:* the `Quote` keeps the interfaces.md 1.9 shape (`lines`, `total_micro_usd`, `budget_micro_usd`, `skipped`); `Quote::calls()` gives the call count instead of a separate `calls` field. `EnrichError` gained `Config` and `Payment`.
- [x] **P3-ENR-02 `NullEnricher`.** `quote` returns 0 calls and 0 USD. `enrich` returns `Err(EgressDisabled)` with the message "enrichment is off: set enrich.backend".
  - *Acceptance:* all playbooks run end to end with Null and report "skipped: no enricher".
  - *Status:* the variant is still `EnrichError::Disabled` (the enrichment.md name); only the message changed.
- [x] **P3-ENR-03 `MockEnricher`.** Records every request and returns canned fixtures from `fixtures/enrich/*.json`. Its quote is configurable.
  - *Acceptance:* used by every playbook and policy test in this phase.
- [x] **P3-ENR-04 `TregEnricher`.** Calls treg's routed tools (`find_email`, `linkedin_profile`, `web_search`, and the company lookup equivalent) over HTTP with one token from config or the environment (`OPENSTANCE_TREG_TOKEN`). Map each `EnrichRequest` to one routed call. Provider choice and fallback stay on treg's side. The quote comes from treg's price information before the call.
  - *Acceptance:* recorded HTTP fixtures (no live calls in CI) cover each request kind and the error paths (401, 402 insufficient balance, timeout, provider failure). One manual live test per release is recorded in the release checklist. The token never appears in logs (a test greps the tracing output).
  - *Status:* quotes come from `GET /catalog/endpoints/<id>` (`max(routing.plan[].usd)`, no token, no contact data); calls send `X-Treg-Route-Max-Cost` equal to the quote. The local-registry check (`crates/openstance/tests/treg_e2e.rs`, `OPENSTANCE_TREG_E2E=1`) ran on 2026-10-21 against dev/treg: every op priced (profile $0.38, find_email $0.15, verify_email $0.024, company $0.38, web_search $0.16), `path --plan` wrote only `quoted` rows, and an approved run without a token ended `partial` with one `error` row and no graph writes. The paid leg with a token, and the per-release live run on a real target (exit criterion 4), are still open.
- [x] **P3-ENR-05 `OwnKeys` stub** behind the `ownkeys` feature flag. Its `quote` returns `Unsupported` and its README lists the planned providers. It exists so the config value is reserved.

### `openstance-core`

- [x] **P3-CORE-01 Egress policy layer.** `EgressGuard` wraps every `&dyn Enricher` call made from a playbook. It (1) drops `no_enrich` people from requests, (2) asks the enricher for a quote, (3) checks `allow_egress` and `min(max_usd, config.max_usd_per_run)`, (4) writes an `enrich_log` row with status `quoted`, `sent`, `ok`, `error`, or `blocked`, and (5) records the actual cost when the backend reports it.
  - *Acceptance:* `MockEnricher` tests check that with `allow_egress=false` there are zero enrich calls and one log row with status `quoted`; that going over budget aborts before the first call that would exceed it; that `no_enrich` people are never present in a recorded request; and that the log row is written even when the call errors.
- [ ] **P3-CORE-02 Writing enrichment results.** Results are written as identities and person fields under a `source` with `kind = 'enrich:<backend>'`. They never overwrite an override, and `rebuild` replays them like any other source.
  - *Acceptance:* a property test applies an override, then enriches the same field, then rebuilds, and the override wins. Enrichment sources show in `who` provenance.
  - *Status (open):* person fields are written under `enrich:<backend>` sources, rank below overrides after `rebuild` (property test in `crates/openstance-core/tests/enrich_commit.rs`) and show in `who` provenance. Found emails and LinkedIn URLs are not yet written as identity rows, and verification status is not stored (needs the resolve layer: an identity with no interactions would become a new person).
- [x] **P3-CORE-03 `Playbook` trait.** `Playbook { name, plan(&Graph, &Value) -> Plan, run(&Graph, &dyn Enricher, &Value) -> Outcome }`. A `Plan` lists fixed steps, the egress each step might make, and the total quote. An `Outcome` holds the typed result plus a cost receipt (from `enrich_log` ids). Steps that have enough local data skip egress, and that is the only branching allowed.
  - *Acceptance:* `plan` never calls `enrich`, which a MockEnricher test checks. The CLI, MCP, and (later) API all call the same functions.
- [x] **P3-CORE-04 `path` playbook.** Steps: (1) look the target up in the graph, and if found, return `how_connected`; (2) if not found, enrich the profile (employers, school); (3) intersect those employers and that school with the graph (current and past `company`, and alumni where known); (4) rank the mutuals by `my_score x tie_weight` (or by `my_score` alone for people at the same company); (5) optionally find and verify the email. Egress is at most 1 profile, 1 find, and 1 verify.
  - *Acceptance:* MockEnricher fixture tests: an in-graph target makes 0 calls; an out-of-graph target makes exactly the planned calls, in order; `--no-email` skips steps 5a and 5b; the quote equals the sum of the step quotes.
- [x] **P3-CORE-05 `brief` playbook.** Steps: (1) the last 5 interactions; (2) mutuals ranked; (3) an optional web search for recent public posts (1 egress); (4) render a markdown brief of at most 10 lines, with sections in a fixed order: who, how you know them, last contact, best mutual, recent public activity (if fetched), and a suggested opener built only from facts.
  - *Acceptance:* a golden markdown file for a fixture person with egress off. Every factual line maps to an interaction id or an enrich result id (included in the JSON form). There is no free-form model prose unless a reranker or explainer is configured (Phase 4).
- [x] **P3-CORE-06 Views: create.** `view create <name> --query '<Query JSON or flags>' [--ttl 30d] [--to <age-recipient>]` stores a `view` row (`query_json`, `created_at`, `expires_at`, `recipient_pubkey`).
  - *Acceptance:* `query_json` is the Phase 1 `Query` serialisation, and it round-trips.
- [x] **P3-CORE-07 Views: export (.ost).** Run the saved query now. Drop `do_not_share` people. Write a temporary SQLite with `person` (name, title, company, location, score *bucket*, last-contact *month*), `identity` (email and LinkedIn only), and `meta` (owner pubkey, created, expires, query_json, view id). Encrypt it with age to the recipient (or to self) and write `name.ost`. No interactions, subjects, notes, tags, or exact scores are included.
  - *Acceptance:* a test decrypts a fixture export and checks the table list and columns against an allow-list. A `do_not_share` person matching the query is absent. The plaintext temp file is created in a private temp dir and deleted on success or failure.
- [x] **P3-CORE-08 Views: import, list, search.** `view import <file.ost>` decrypts with `~/.openstance/age.key`, refuses files past `expires_at`, and stores them under `~/.openstance/views/<view_id>.db`, keeping only the latest copy per view id. `view list` lists them. `view search <name> "<text>"` runs FTS (and vector, if embedded locally on import) over the imported view.
  - *Acceptance:* an expired view is refused with a clear message. Re-importing a newer export of the same view id replaces the old one. Imported view people never mix into the owner's `person` table.
  - *Status:* search is FTS only (imported views are not embedded). Imported views live as files under `~/.openstance/views/` and are not registered as `view` rows in graph.db.
- [x] **P3-CORE-09 age keypair.** Generate an X25519 identity at `~/.openstance/age.key` (mode 0600) on first need, or during `init` for new installs. `openstance keys show` prints the public key to share.
  - *Acceptance:* the key file permissions are checked in a test on Unix. An existing key is never overwritten.

### `openstance-import`

- [x] **P3-IMP-01 `ics` importer.** Each VEVENT with attendees becomes a `meeting` interaction (`subject = SUMMARY`, `external_id = UID + RECURRENCE-ID/occurrence date`). Attendee emails are identities at confidence 1.0. Expand recurring events only for occurrences in the past, capped per series (the cap is set in [ingestion](../tech-spec/ingestion.md)). Skip events with no attendee other than the owner, and declined events.
  - *Acceptance:* fixture tests for Google and Apple exports, recurring meetings, timezones (TZID and floating), and cancelled events. Meetings feed the M component and the ties.
  - *Status:* RDATE is not expanded (RRULE with EXDATE and RECURRENCE-ID overrides is). The past/future cut uses the system clock, not `--now`.
- [x] **P3-IMP-02 `x_archive` importer.** Read `following.js`, `follower.js`, and `direct-messages.js` from the zip (strip the `window.YTD... =` prefix). Emit `x_handle` identities, `follow` interactions, and `dm` interactions with direction. Handles link to existing people only through an exact handle identity, a name match flagged at the normal name confidence, or a user link (`openstance edit <p> --x-handle`).
  - *Acceptance:* the fixture archive gives the expected follow and dm counts. No auto-merge happens on a name match without the usual rules.

### `openstance-mcp`

- [x] **P3-MCP-01 `path_to`.** Input `{ target, company?, linkedin?, find_email?, limit?, allow_egress=false, max_usd?, plan_only? }`. Output: the `Outcome` shape (`plan` with `quote`, `result`, `receipt`) from [interfaces](../tech-spec/interfaces.md#19-quote-receipt-outcome). With `allow_egress=false` it returns the plan and quote only.
- [x] **P3-MCP-02 `brief`.** Input `{ person, web=false, allow_egress=false, max_usd? }`. Output markdown plus structured facts. Without egress, the web search step is skipped and reported.
- [x] **P3-MCP-03 `quote_enrich`.** Input: `{ requests: [...] }` (one or more enrich requests). Output: a `Quote`. Never spends.
- [x] **P3-MCP-04 `list_views`, `search_view`.** `list_views` covers own and imported views (`direction` filter); `search_view` searches imported, unexpired views (or `*` for all).
- [x] **P3-MCP-05 Skills** `playbooks/path.md` and `playbooks/brief.md`. They tell the agent to call with `allow_egress=false` first, show the user the quote, and only call again with `allow_egress=true` after the user explicitly agrees. Written in our own words with no third-party promotion.
  - *Acceptance (all MCP):* schema snapshot tests. An agent session with MockEnricher shows the quote-then-confirm flow. The default call never spends.
  - *Status:* `crates/openstance-mcp/tests/docs.rs` lints every skill (frontmatter, registered tools only, no URLs, no third-party names). `tests/playbook_tools.rs` runs the quote-then-confirm flow with MockEnricher; `tests/mcp_playbooks.rs` runs it through the binary against a treg-shaped local server.

### `openstance` (CLI)

- [x] **P3-CLI-01 `path "<target>" [--company X] [--allow-egress] [--max-usd 0.10] [--find-email]`.** Prints the plan with per-step costs and the total. Without `--allow-egress` (alias of `--yes`) it prompts on a TTY, and otherwise stops there with "re-run with --allow-egress to spend $0.0xx".
- [x] **P3-CLI-02 `brief <person> [--allow-egress]`.** Prints the markdown to stdout (pipe-friendly).
- [x] **P3-CLI-03 `view create|export|import|list|search`** and `keys show`.
- [x] **P3-CLI-04 Egress log viewer.** `openstance enrich log [--since 7d] [--json]` lists every request, what was sent, and its cost. Use the exact command name defined in [interfaces](../tech-spec/interfaces.md).
  - *Status:* `enrich quote` and `enrich forget` (interfaces.md section 8) are not built; MCP `quote_enrich` and `Graph::quote_enrich` cover pricing.
- [x] **P3-CLI-05 `export --format vcf`.**
  - *Acceptance (all CLI):* `--json` output matches the MCP output for the same inputs.

### Parallelisation

Streams: (1) enrich + egress policy + playbooks (ENR-*, CORE-01..05, MCP-01..03, CLI-01/02/04); (2) views + keys (CORE-06..09, MCP-04, CLI-03); (3) importers (IMP-01/02). Stream 3 is a good fit for an outside contributor if one shows up after Show HN. Label it `good-first-issue`.

---

## Deliverables

- `v0.4.0` tag. Commands added: `path`, `brief`, `view ...`, `keys show`, `enrich log`, `export --format vcf`.
- MCP tools: `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich` (8 tools in total).
- `playbooks/path.md` and `playbooks/brief.md`.
- `ics` and `x_archive` importers with fixtures.
- Follow-up post or HN comment: "path to a stranger, cents per call, quote first".

## Demo script (Phase 3, about 90 seconds)

```sh
# 1. Not in my graph: plan and quote only, nothing sent
ost path "Dana Whitfield" --company "Lumen Pay"
#  step 1 lookup in graph ............ not found
#  step 2 enrich profile ............. treg  ~$0.010
#  step 3 intersect employers/school . local $0
#  step 4 rank mutuals ............... local $0
#  step 5 find + verify email ........ treg  ~$0.006
#  total ~$0.016 (budget $0.50). Re-run with --allow-egress to spend.

# 2. Run it
ost path "Dana Whitfield" --company "Lumen Pay" --allow-egress
#  -> 3 mutuals who worked at Orbit Labs with Dana; best: Maya Chen (score 0.81)
ost enrich log --since 1h          # exactly what left the machine, and what it cost

# 3. Brief for the ask
ost brief "Maya Chen" > ask.md

# 4. Share a slice, not the graph
ost view create devtools-investors --query '{"title":["investor"],"text":"devtools"}' --ttl 30d --to age1q...
ost view export devtools-investors > devtools-investors.ost
#  (colleague) ost view import devtools-investors.ost && ost view search devtools-investors "seed"
```

Then do the same `path_to` flow in Claude Code. The agent shows the quote and asks before spending.

## Exit criteria

1. All P3 tasks are closed or moved with a link.
2. Policy tests are green: no egress with the defaults; budget abort; `no_enrich` never sent; `enrich_log` row on every attempt; override beats enrichment after a rebuild.
3. A view export contains only the allow-listed tables and columns, never a `do_not_share` person, and an expired view is refused on import.
4. One manual live treg run of `path` on a real out-of-graph target costs no more than the quote, and the `enrich_log` actual cost matches treg's receipt.
5. The ics and x_archive fixtures pass. The resolution counts in `fixtures/EXPECTED.md` are updated on purpose.
6. A fresh-install check confirms that with an empty config, no command (including `path` and `brief`) opens a network socket. Verify by blocking the network in an integration test.

Status (2026-10-21, branch `build/phases-1-3`):

1. Open: P3-CORE-02 is partial (identity rows from enrichment). Every other task is closed.
2. Met. The policy tests are in `crates/openstance-core/src/egress/tests.rs`, `src/playbook/tests.rs` and `tests/enrich_commit.rs`.
3. Met (`crates/openstance-core/src/view/` tests and `crates/openstance/tests/cli_views.rs`).
4. Open. Only the local-registry dry run was done: quotes only, no spend (see P3-ENR-04). The paid run on a real target needs a person with a funded token.
5. Met (`fixtures/gen/tests/phase3.rs`; `fixtures/EXPECTED.md` has a Phase 3 section).
6. Met on macOS and Linux (`crates/openstance/tests/no_network.rs`, using `sandbox-exec` or `unshare -rn`; skipped elsewhere).

The demo script runs end to end on `fixtures/demo` in `crates/openstance/tests/demo_phase3.rs` (`-- --nocapture` prints the transcript). Paid steps go to a canned treg-shaped local server. The dataset already contains Dana Whitfield, so the stranger is "Noor Haddad" at Lumen Pay; the view is `fintech-investors` (`title: Venture Partner | Principal`). The `v0.4.0` tag and the follow-up post are not done.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| treg is a young dependency (API changes, outage) | medium | medium | It is one implementation behind the `Enricher` trait. HTTP fixtures pin the contract. Null keeps everything working. OwnKeys is reserved. treg is open source, so self-hosting the router is possible |
| Egress weakens the privacy pitch | medium | high | Off by default, quote first, `enrich log` audit, `no_enrich`, and clear docs on exactly which fields are sent per request kind |
| Quotes do not match actual cost | medium | medium | Record actual costs. Use a conservative quote (round up). Hard budget ceiling in core |
| Views leak more than intended | low | high | A column allow-list test, no interactions, buckets instead of exact scores, and `do_not_share` respected. Expiry is advisory once the file has been delivered, and the docs say so |
| ics recurrence expansion inflates meeting counts | medium | medium | Cap per series, count past occurrences only, M saturates (`1 - exp(-n/3)`) |
| Scope creep into the Phase 4 playbooks | high | low | `refresh`/`verify` share code with `path` but stay unmerged until Phase 4 |
