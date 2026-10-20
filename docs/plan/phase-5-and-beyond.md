# Phase 5 and beyond: sync, teams, more sources, and the cloud decision

**Timebox:** none. Each work package is planned when it is pulled. **Gated on:** passing [Gate B (park criterion)](roadmap.md#gate-b-park-criterion-show-hn--1-month). The cloud tier is also gated on [Gate C](roadmap.md#gate-c-cloud-decision-about-4-weeks-after-v10-only-if-gate-b-passes). **Previous:** [Phase 4](phase-4.md). **Overview:** [roadmap](roadmap.md).

## Goal

Grow where users pull, in this order of evidence: the importers people ask for, multi-device and team use of views, and only then hosted conveniences that someone will pay for. Throughout, the free local product stays complete. Nothing that works locally today moves behind a paywall.

## Entry conditions

Phase 5 starts only when all of these are true:

1. v1.0 is released and the [Phase 4 exit criteria](phase-4.md#exit-criteria) are met.
2. Gate B has passed: there was a meaningful response to the launch (see [launch.md](launch.md#park-criterion)).
3. The issue tracker has been triaged into the work packages below, with the number of non-maintainer requests for each recorded. The packages are then ordered by that demand, not by the order in this document.

If Gate B fails, the project is parked. See [roadmap](roadmap.md#gate-b-park-criterion-show-hn--1-month).

---

## Work packages

Each package below is self-contained, so packages can be built in any order and in parallel by different contributors. The task IDs follow the `P5-<AREA>-<nn>` scheme.

### WP1: more importers (`openstance-import`)

Importers are the cheapest way to add value and the best way to bring in contributors. Each one is a crate module behind a cargo feature, has a synthetic fixture, and comes with a test that updates `fixtures/EXPECTED.md`. See [ingestion](../tech-spec/ingestion.md) for the trait and the per-importer rules.

- [ ] **P5-IMP-01 `whatsapp`.** Chat export `.txt` (Android and iOS formats, many locales and date formats). Emits `dm` interactions keyed by phone (E.164) or name. Names only, unless a vcf phone identity links them.
  - *Acceptance:* locale fixtures (en-US, en-GB, en-IN, de-DE at minimum). Group chats emit one interaction per message day with all participants, so ties are not inflated message by message. Message text is never stored. Only a subject-like first line is kept, and only if `import.email_bodies`-style opt-in is set (`import.message_text = false` by default).
- [ ] **P5-IMP-02 `slack`.** Workspace export zip (`users.json`, `channels.json`, per-day message files). Emits identities (email when present, Slack user id) and `dm` / `mention` interactions.
  - *Acceptance:* large-channel noise is capped (channels with more than N members contribute ties only through mentions and threads, not through simple co-membership).
- [ ] **P5-IMP-03 `discord`.** Data package (DMs, servers, and friends where present). Emits handle identities and dm interactions.
- [ ] **P5-IMP-04 `telegram`.** Desktop export `result.json`. Emits dm interactions and phone and username identities.
- [ ] **P5-IMP-05 `notes`.** Granola, Obsidian, and Apple Notes markdown exports. A note becomes an interaction (`kind = mention`, `subject = note title`) linked to the people it mentions, through `[[wikilinks]]`, emails, or exact name matches at the normal name confidence. Note bodies are not stored.
- [ ] **P5-IMP-06 `imap`.** The same headers as mbox, read over IMAP with app passwords, incremental by UID with a stored cursor per mailbox. `openstance import imap://...` or a `[[import.imap]]` config block. Credentials go in the OS keychain where available.
  - *Acceptance:* a second run only fetches new UIDs. A UIDVALIDITY change triggers a full re-scan of that mailbox (deduped by Message-ID).
- [ ] **P5-IMP-07 `pst` / `eml`.** Outlook PST and EML, headers only.
- [ ] **P5-IMP-08 `csv` with a column map.** Any CSV plus a TOML column map (`--map crm.toml`), for CRM exports and event attendee lists. Reuse the CSV edge-case tests from the [v0.1 prototype](../prototype-v0.1.md). Because this is small and comes up often, it can be pulled forward into any phase as a `good-first-issue`.

### WP2: sync over git or S3 (`openstance-core`, new `sync` module)

Sync moves **encrypted views** between people and machines. It never moves the graph database itself. See [sharing and views](../tech-spec/sharing-and-views.md).

- [ ] **P5-SYNC-01 Remote abstraction.** A `Remote` trait with `git` (shells out to `git`, or uses `gix`) and `s3` (any S3-compatible bucket) implementations. Configured under `[sync] remote = "git@..."` or `"s3://bucket/prefix"`.
- [ ] **P5-SYNC-02 Layout.** `views/<recipient-pubkey-fingerprint>/<view-id>.ost` plus a signed `index.json` per sender. Files are age-encrypted, so the remote never sees plaintext.
- [ ] **P5-SYNC-03 `openstance sync`.** Push the views you have exported (re-exported on each run so expiry and query changes apply), and pull views addressed to your public key, importing them through the Phase 3 importer (expiry is enforced there).
  - *Acceptance:* two-user integration test with a local bare git repo and MinIO. Revocation is "stop re-exporting and delete the remote file", and the docs say plainly that a copy already pulled cannot be recalled.
- [ ] **P5-SYNC-04 Personal multi-device mode (stretch).** Sync an encrypted *backup* of the graph between your own devices, using the same remotes. It is age-encrypted to your own key. Conflicts are resolved by "sources and overrides are append-only, then rebuild". This is the free, bring-your-own-remote version of the cloud tier's sync.
  - *Acceptance:* an override made on device A and an import made on device B both survive after sync and rebuild on both devices.

### WP3: team federation (`openstance-core`, `openstance-mcp`)

Team mode means sync with a shared remote. Each member keeps their own file and exports only the views they choose. Search federates over the imported views at query time. There is no central database. The authorization rules in the [research notes](../prototype-v0.1.md) apply: unshared records must not leak through counts, embeddings, explanations, or caches.

- [ ] **P5-TEAM-01 Federated search.** `search --views all|<names>` and an `include_views` input on `search_people` merge local results with view results, labelled with their origin (`via view "devtools-investors" from alice`).
  - *Acceptance:* a view person never contributes to local scores or ties. Result counts reveal nothing about people who are not in a view.
- [ ] **P5-TEAM-02 Team "who knows X".** For a company or target, list which teammate's view covers it, with the score *bucket* and the last-contact month only, as exported.
- [ ] **P5-TEAM-03 Intro request etiquette.** Recipient-interest states (`asked`, `willing`, `declined`, `do not contact`) on a view entry, stored locally by the view owner and re-exported. This builds on the v0.1 intro board concepts (shortlist, asked, introduced, meeting).
  - *Acceptance:* a `declined` or `do not contact` state hides the person from teammates' federated results on the next sync.
- [ ] **P5-TEAM-04 Audit.** A local log of which views were exported to whom and when, viewable with `openstance view log`.

### WP4: optional OAuth connectors (`openstance-import`)

Export-first stays the default. OAuth exists for people who want live sync and are willing to bring their own OAuth app.

- [ ] **P5-OAUTH-01 Gmail (bring your own OAuth client).** Metadata or readonly scope, chosen to read headers only. The scope and verification requirements must be checked against Google's current documentation when this is built. The user registers their own Google Cloud OAuth client. openstance ships no client secret.
- [ ] **P5-OAUTH-02 Outlook / Microsoft Graph (bring your own app).** Same model as Gmail.
- [ ] **P5-OAUTH-03 Google Contacts.** People API, read-only.
  - *Acceptance (all):* incremental cursors, disconnect deletes the tokens, `openstance source remove <id>` followed by a rebuild removes all derived data, and tokens are stored in the OS keychain. Managed OAuth (where openstance carries the app verification) is a cloud-tier item and is not built here.

### WP5: product follow-through (candidates, not committed)

These come from the v0.1 prototype and the research notes. Each one needs user demand before it is built.

- Intro tracking (shortlist, asked, introduced, meeting, outcome) ported from the v0.1 intro board to the Rust core as an `intro` table plus CLI and UI.
- Freshness states ("please reconfirm"), conflicting titles across sources, shown in `who`.
- A Tauri desktop shell around the existing UI.
- An SQLCipher build flag, tested in CI.

---

## Cloud tier decision

The cloud tier is **not** planned work. It is a decision taken at [Gate C](roadmap.md#gate-c-cloud-decision-about-4-weeks-after-v10-only-if-gate-b-passes) and recorded as an ADR in [decisions](../decisions/README.md). The commercial reasoning and the indicative prices are in [pricing and cloud](../product/pricing-and-cloud.md).

### Rules that bind any cloud tier

1. No feature works only in the cloud, apart from multi-device sync and hosted endpoints. Everything else (search, importers, scoring, playbooks, MCP, API, views, team mode over your own git/S3) stays free and local.
2. Synced files are encrypted on the client. The service cannot read a user's graph.
3. A cloud user can export and leave with one command. Data export is never a paid feature.
4. There is no pooled, multi-tenant contact database. Team features federate over views.
5. Enrichment is pass-through at cost plus a stated margin, with per-person cost caps, and is never required.
6. Decide the server-side licence before accepting contributions to any server component. Existing MIT releases stay MIT. (This is a product recommendation, not legal advice.)

### Candidate cloud components, in suggested build order

| # | Component | Builds on | Why this order |
|---|---|---|---|
| 1 | Hosted encrypted sync (personal, end-to-end) | WP2 / P5-SYNC-04 | The most common reason to pay. Mostly storage plus key handling |
| 2 | Hosted MCP endpoint | Phase 4 MCP over HTTP | Lets claude.ai, ChatGPT, and phone agents query without your laptop on. Needs a server-side decryption design (for example, a user-held key in a confidential runtime, or a reduced query-only replica the user chooses), which needs its own ADR |
| 3 | Hosted view sharing (links, TTL, revocation, access log) | WP2 | Enforces the TTL and revocation that offline files can only advise |
| 4 | Scheduled playbooks (reconnect digest, watch, refresh) delivered to email or Slack | Phase 4 playbooks | A local cron does this for free. The cloud adds reliability and delivery |
| 5 | Managed enrichment wallet | Phase 3 `Enricher` | Prepaid balance routed at cost plus a margin, one invoice |
| 6 | Team workspace (federated search, SSO, admin, audit) | WP3 | Only after several teams use federation over their own remotes |
| 7 | Managed OAuth connectors | WP4 | Highest compliance cost (provider app verification). Last |
| 8 | Hosted rerank/explain, web and mobile UI over the synced file | Phase 4 adapters, UI | Convenience for people who do not want to run a model |

### Analytics storage if teams need it

The local product stays SQLite. If a large team ever needs cross-member analytics ("which teams have warm coverage into a list of accounts"), the order is: Postgres with materialised views, then DuckDB over the Phase 4 Parquet exports, and ClickHouse only if a customer with more than 100M interactions shows up. The Parquet export keeps this option open without committing to it now.

---

## Exit criteria (per work package)

Each work package closes when:

1. Its tasks are closed, and its fixtures and tests are in CI.
2. The privacy defaults are unchanged, or changed through an ADR.
3. The docs are updated: [ingestion](../tech-spec/ingestion.md) for importers, [sharing and views](../tech-spec/sharing-and-views.md) for sync and teams, and [interfaces](../tech-spec/interfaces.md) for new commands and tools.
4. A release note says plainly what data the feature reads and where that data can go.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Importer maintenance burden (formats change) | high | medium | Each importer has its own fixtures and is behind a feature flag. Mark importers that lose their maintainer as unmaintained instead of letting them rot silently |
| Federation leaks unshared data through counts or embeddings | medium | high | Enforce authorisation in retrieval, not only in display. Add a leakage test suite before WP3 ships |
| OAuth verification cost and policy changes | medium | medium | Bring-your-own-app only in OSS. Managed connectors only in cloud, and only if Gate C says so |
| The cloud tier pulls focus from the OSS core | medium | high | Gate C is a written decision. The cloud rules above are non-negotiable. Keep OSS releases on a regular cadence |
| Happenstance's group reach and live connectors are hard to match | high | medium | Do not compete on reach. Compete on ownership, evidence, free search, and agents (see [competitive analysis](../product/competitive-analysis.md)) |
