# Risks and open questions

Living register for openstance. Review it at every phase gate ([plan/roadmap.md](plan/roadmap.md)). When an open question is decided, record the decision as an ADR in [decisions/](decisions/README.md) and mark the question **Decided** here with a link.

Scales: **Likelihood** Low / Medium / High within the next three months. **Impact** Low / Medium / High on the project's ability to launch and be trusted. **Owner phase** is the phase in which the mitigation must be in place.

## Risk register

### Product and market

| # | Risk | Likelihood | Impact | Mitigation | Owner phase |
|---|---|---|---|---|---|
| R1 | No validated demand. The user's own upgrade prompt is real motivation, but there are no customer interviews or hands-on paid comparisons yet ([prototype research](prototype-v0.1.md#what-to-keep)). | High | High | Ship the smallest useful demo (Phase 2), launch where the paywall complaint lives, and apply the kill/park criterion (R3). Run the 8 to 12 founder pilot with 20 to 30 real questions each in parallel with Phase 2/3. | 2 |
| R2 | Happenstance's moat is live connectors, groups and a large user base; a local tool cannot replicate that. | High | Medium | Do not compete on groups, iOS or connector breadth in v1. Win on free unlimited search, a file the user owns, explained scores and agent access. Federated team views later (Phase 5). See [product/competitive-analysis.md](product/competitive-analysis.md). | 2 |
| R3 | One more project on a full plate; scope creep delays launch. | High | High | Scope is one binary and one demo. **Kill/park criterion:** no meaningful Show HN response within a month of launch, park it. Define "meaningful" before launch (see Q12). | 2 |
| R4 | Trademark complaint if the name or copy leans on "Happenstance". | Medium | High | ADR-0001 decides the name before launch; factual comparison on one page; no parody branding, no borrowed logos or colors. | 2 |
| R5 | Competitive claims go stale or are wrong (pricing, free-tier limits, privacy controls). | High | Medium | Date every claim; cite the vendor's own page; the "~15 searches/month free" figure is third-party and **unconfirmed**, so do not state it as fact. Pricing per their page: $24/mo billed annually, $30 monthly. Do not claim Happenstance lacks privacy controls or evidence. Re-verify in the [launch checklist](plan/launch.md). | 2 |
| R6 | Export-first onboarding is too slow (Google Takeout can take hours to prepare). | Medium | Medium | Lead the quickstart with LinkedIn `Connections.csv` and `.vcf` (minutes), then mbox. Document one-minute export steps per source. | 1 |
| R7 | Data goes stale between exports. | High | Low | `watch` diffs LinkedIn exports; `refresh` enriches stale rows on request; show last-contact and source dates in every result. | 3 to 4 |

### Technical

| # | Risk | Likelihood | Impact | Mitigation | Owner phase |
|---|---|---|---|---|---|
| R8 | Entity resolution mistakes are visible and destroy trust. | Medium | High | Conservative rules ([ADR-0009](decisions/0009-conservative-entity-resolution.md)); never merge on name alone; flagged merges in `openstance review`; merge/split overrides survive rebuild; fixture tests assert exact person and flagged counts. | 1 |
| R9 | Search quality is weak without a model or without email bodies. | Medium | High | 50-query eval with expected top-5 ids in CI (rules parser, no reranker); measure headers-only quality in Phase 2 before revisiting [ADR-0010](decisions/0010-email-headers-only-by-default.md); subjects in FTS and embeddings. | 2 |
| R10 | Small local models produce poor prose explanations or bad query parses. | High | Low | Deterministic evidence lines by default; model output validated against `Query` with fallback to `RuleParser`; extracted filters are shown and editable. | 2 |
| R11 | Packaging native pieces (bundled SQLite, sqlite-vec, ONNX runtime) for all targets is fiddly; binary size exceeds 40 MB. | Medium | Medium | CI matrix builds release artifacts from Phase 1, not Phase 4; models downloaded on first use and cached; feature flags allow a `--no-default-features` build without embeddings. | 1 |
| R12 | Rust velocity is slower than planned. | Medium | Medium | Keep core sync; small traits; if Phase 1 slips more than a week due to Rust-specific friction, revisit per [ADR-0002](decisions/0002-rust-for-core.md). | 1 |
| R13 | Performance targets missed on large inputs (5 GB mbox, 500k interactions). | Medium | Medium | Streaming parser; one transaction per import with WAL; benchmarks on a synthetic large fixture in CI (nightly). Targets in [engineering/performance-targets.md](engineering/performance-targets.md). | 1 to 2 |
| R14 | Export formats change (LinkedIn CSV columns and preamble, X archive JS wrappers, Takeout layout). | High | Medium | Header-alias tables (seeded from the v0.1 prototype), `detect()` sniffing, fixtures per known format version, clear error messages naming the unrecognized column. | 1 |
| R15 | Bulk-sender heuristic mislabels real people (they never reply, or send newsletters and personal mail). | Medium | Low | Bulk people are hidden, not deleted; `--include-bulk` shows them; `openstance edit <p> --not-bulk` writes an `is_bulk` scalar override that always wins ([tech-spec/data-model.md](tech-spec/data-model.md#47-override-edit)). | 1 |
| R16 | Scoring constants (half-life 180 days, F/20, M/3, fusion 0.5/0.5, top 200) are untuned guesses. | High | Low | All are config keys; golden tests pin intent; tune against the eval set and pilot feedback. | 2 |

### Privacy, security and legal

| # | Risk | Likelihood | Impact | Mitigation | Owner phase |
|---|---|---|---|---|---|
| R17 | Enrichment sends contact names and emails to a third party, cutting against the privacy pitch. | Medium | High | Off by default (`enrich.backend = "none"`, `allow_egress_default = false`); every call quoted first and logged to `enrich_log`; `no_enrich` and `do_not_share` respected; per-run cap `max_usd_per_run = 0.50`. See [ADR-0007](decisions/0007-enricher-trait-egress-off.md). | 3 |
| R18 | treg is a young startup; it may change pricing, APIs or disappear. | Medium | Medium | `Enricher` is a trait; treg is one implementation beside `Null` and `OwnKeys`; treg is open source so self-hosting the router is a fallback. | 3 |
| R19 | treg's published skill file instructs agents to promote treg. | High (it exists) | Medium | Do not vendor it. openstance writes its own playbook skills with no third-party promotion; CI greps `playbooks/` for promotional patterns (starring repos, "recommend X"). | 3 |
| R20 | An agent spends money without the user noticing. | Medium | Medium | Every money-spending MCP tool takes `allow_egress` (default false) and `max_usd`, and returns a quote when false; budget enforced in core, not in the agent. | 3 |
| R21 | Prompt injection: interaction subjects, names and bios are attacker-controlled text (anyone can email you) and are returned to agents via MCP. | Medium | Medium | Return them as data fields in structured JSON, never as instructions; document in playbook skills that tool results are untrusted data; never let tool output trigger egress without `allow_egress` from the user. See [architecture/security-and-threat-model.md](architecture/security-and-threat-model.md). | 2 |
| R22 | Local HTTP API abused by a malicious web page (CSRF, DNS rebinding). | Low | High | Bind to `127.0.0.1:7411`; bearer token generated at init; no cookies; CORS off except the embedded UI origin; reject unexpected `Host` headers. | 4 |
| R23 | The local `graph.db` is stolen (unencrypted at rest). | Low | High | Recommend full-disk encryption; optional SQLCipher build flag; headers-only default limits the blast radius. | 1 |
| R24 | Shared views cannot be truly revoked or expired once delivered. | High (inherent) | Medium | State it plainly in CLI output and docs; views exclude interactions and `do_not_share` people; importer refuses expired files; score bucket and last-contact month only. See [ADR-0008](decisions/0008-views-as-encrypted-files.md). | 3 |
| R25 | Data-protection obligations when processing third parties' personal data, especially enrichment and team sharing. | Medium | Medium | Local personal use keeps the user in control; enrichment and sharing are explicit and logged; per-person `no_enrich`/`do_not_share`; get legal review before any cloud tier. Not legal advice. | 5 |
| R26 | Google OAuth restricted-scope verification if live Gmail sync is added. | High (if built) | Medium | Export-first for v1 ([ADR-0004](decisions/0004-export-first-ingestion.md)); bring-your-own OAuth app in OSS; managed connectors only as a later cloud feature. | 5 |

## Open questions

| # | Question | Options | Current lean | Decide by | Status |
|---|---|---|---|---|---|
| Q1 | **Product name** | openstance, kith, others ([ADR-0001](decisions/0001-product-name.md)) | kith, if crates.io, domain and trademark checks pass | Before Phase 2 launch | Open |
| Q2 | **Email bodies by default?** Bodies improve search and title/company coverage; they worsen the privacy story, file size and import time. | headers only (current); bodies opt-in; signatures-only default | Headers only; revisit with Phase 2 eval numbers ([ADR-0010](decisions/0010-email-headers-only-by-default.md)) | End of Phase 2 | Open |
| Q3 | **Ship the web UI in v1 at all**, or stay CLI + MCP until people ask? | minimal UI in Phase 4; no UI until requested | Minimal read/edit/merge UI in Phase 4 only if Phase 2 feedback asks for it; otherwise move to Phase 5 | Phase 3 gate | Open |
| Q4 | **Enrichment wallet margin** for a future cloud tier. | at cost; small fixed fee per call; percentage margin | Undecided; no revenue projection is justified yet. See [product/pricing-and-cloud.md](product/pricing-and-cloud.md) | Before cloud tier | Open |
| Q5 | **Licence for the Rust core:** MIT (like the v0.1 prototype) or Apache-2.0 (explicit patent grant). | MIT; Apache-2.0; dual MIT/Apache-2.0 (Rust ecosystem norm) | MIT, matching the prototype and the launch copy in [plan/launch.md](plan/launch.md); dual MIT OR Apache-2.0 (the Rust norm, adds a patent grant) remains the alternative. Record as an ADR before accepting outside contributions. Not legal advice. | Before first public commit | Open |
| Q6 | **Hosted MCP vs end-to-end encryption** for a cloud tier. | relay to user's device; opt-in server-side decryption; hosted MCP over shared views only | Relay + views-only (see [product/pricing-and-cloud.md](product/pricing-and-cloud.md)) | Before cloud tier | Open |
| Q7 | **Is the intro workflow in scope?** The v0.1 prototype has an intro board (Shortlisted, Asked for intro, Introduced, Meeting booked) and the research argues follow-through is a differentiator; the tech spec has no intro table. | port as tags/notes; add an `intro` table post-v1; drop | Not in Phases 1 to 4; use tags (`intro:asked`) as a stopgap; revisit after launch feedback | Phase 5 | Open |
| Q10 | **SQLCipher**: optional build flag only, or default-on with an OS keychain? | flag only; default-on | Flag only for v1 | Phase 4 | Open |
| Q11 | **Telemetry**: any at all? | none; opt-in anonymous counts | None. Measure via GitHub stars, downloads (cargo-dist/Homebrew counts) and issues | Phase 2 | Lean decided, needs ADR |
| Q12 | **What counts as a "meaningful Show HN response"** for the kill/park criterion? | star/point thresholds; qualitative usage signals | The proposal in [plan/launch.md](plan/launch.md#park-criterion): continue if at least two of (10 non-maintainer users on their own exports, 3 external PRs, 3 "forgotten person found" reports, team interest from 3 groups) within 30 days. Stars alone are not enough. Confirm in an ADR before posting. | Before launch | Open |
| Q13 | **Platform support tier for Windows.** | tier 1; best-effort | macOS and Linux tier 1; Windows built by cargo-dist, best-effort until someone uses it | Phase 4 | Open |
| Q14 | **Multiple graphs per machine** (work vs personal). | `--db` flag only; named profiles | `--db` flag (already in the CLI); profiles later | Phase 5 | Open |
| Q15 | **Pilot timing.** The research proposes 8 to 12 founders with consented exports, 20 to 30 real questions each, with gates (useful shortlist under 5 minutes; 80% top-5 relevance; no unsupported facts; no data loss). | before launch; after launch | Recruit during Phase 2; run during Phase 3 | Phase 2 | Open |
| Q16 | **Cloud tier at all?** | none; sync-only; full cloud | Decide after Phase 5 based on who shows up | Post-launch | Open |
| Q17 | **Signed views.** age does not authenticate the sender, so a view's `owner_label` is only a claim ([tech-spec/sharing-and-views.md](tech-spec/sharing-and-views.md)). | accept as documented; Ed25519 signature over the `.ost` payload | Add a signature in Phase 5 with team federation | Phase 5 | Open |
| Q18 | **Deleting one third party's data on request.** There is no single command that removes a person's identities and interactions from every source while keeping the rest of each source. | `openstance forget <person>` that tombstones identities and drops their participant rows; rely on `source rm` + re-import | Design a `forget` command before any cloud tier; local use is covered by `edit --no-share --no-enrich` and `enrich forget` | Phase 5 | Open |
| Q19 | **Passphrase-protected `age.key`.** The view identity is a plain 0600 file. | plain file (current); age passphrase-encrypted identity; OS keychain | Plain file in v1; keychain alongside the SQLCipher option | Phase 4 | Open |
| Q20 | **S3 client for Phase 5 sync.** Must be blocking so tokio stays out of core. | blocking client over `ureq` + SigV4; `rust-s3` blocking feature; shell out to `aws` | Decide at Phase 5 start | Phase 5 | Open |
| Q21 | **Model management commands** (`openstance models list|pull|rm`) proposed in [tech-spec/models.md](tech-spec/models.md) but not in [tech-spec/interfaces.md](tech-spec/interfaces.md). | add in Phase 4; keep `rebuild --embeddings` only | Add in Phase 4 if model choices grow beyond one embedder | Phase 4 | Open |
| Q22 | **Scoring refinements**: group dampening for F, large-meeting dampening for M, a `noreply@`/`notifications@` bulk rule, and two-hop paths for `path_to` ([tech-spec/scoring.md](tech-spec/scoring.md#10-tuning-and-open-questions)). | each independently | Tune after the Phase 2 eval exists; two-hop paths no earlier than Phase 3 | Phase 2 to 3 | Open |
| Q23 | **macOS signing and notarization** of release binaries ([engineering/ci-and-release.md](engineering/ci-and-release.md)). | unsigned + Homebrew; Developer ID signing | Unsigned for Show HN (Homebrew and `cargo binstall` avoid Gatekeeper prompts); sign by v1.0 if users hit quarantine prompts | Phase 4 | Open |
| Q24 | **Remote MCP connectors** (claude.ai, ChatGPT) need a public HTTPS URL and may require OAuth; a loopback server cannot serve them ([tech-spec/interfaces.md](tech-spec/interfaces.md#23-client-setup)). | user-run tunnel + `serve --mcp-only`; hosted MCP in a cloud tier | Document the tunnel path in Phase 4 after verifying each connector's auth; hosted MCP is cloud-tier work (see Q6) | Phase 4 | Open |

## Decided (moved out of this list)

| Question | Decision |
|---|---|
| Storage engine (SQLite vs ClickHouse) | SQLite + FTS5 + sqlite-vec: [ADR-0003](decisions/0003-sqlite-fts5-sqlite-vec.md) |
| Separate `who_knows` MCP tool | No; `search_people` + `how_connected`: [ADR-0011](decisions/0011-one-json-shape-across-surfaces.md) |
| MVP length (2 vs 4 weeks) | Phase 1 + 2 = public MVP; Phases 3 and 4 complete v1: [ADR-0012](decisions/0012-spec-corrections.md), [plan/roadmap.md](plan/roadmap.md) |
| Spec inconsistencies (table count, bulk flag, tie evidence, recency, overrides, FTS type, default weights) | [ADR-0012](decisions/0012-spec-corrections.md) |
| Correcting bulk-sender misclassification (was Q8) | `is_bulk` is an extra scalar override field; `openstance edit <p> --not-bulk`. [tech-spec/data-model.md](tech-spec/data-model.md#47-override-edit) |
| Generic CSV importer with column map (was Q9) | Phase 5 (P5-IMP-08), labelled good first issue, may be pulled earlier. [plan/phase-5-and-beyond.md](plan/phase-5-and-beyond.md) |
| Hosted model calls (parser, reranker, embedder) | Treated as egress: opt-in by config, one-time notice, logged to `enrich_log` as `model:<provider>`, bulk embedding quoted, `no_enrich` people never sent. [tech-spec/models.md](tech-spec/models.md#9-privacy-and-egress-rules-for-models) |
| Default score preset | A `default` preset (R .35 F .20 C .25 M .15 B .05) applies with an empty config; `founder` keeps the spec's .30/.15/.30/.20/.05. [tech-spec/scoring.md](tech-spec/scoring.md#4-presets) |
