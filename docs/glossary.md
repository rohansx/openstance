# Glossary

Terms used across the openstance docs, in alphabetical order. Table and column names are in `code`. Where a term has a precise definition in a spec doc, the entry links to it.

| Term | Definition |
|---|---|
| **`accounts`** | Playbook. For each company in a list: current people in your graph, alumni (from LinkedIn history), and one-hop coverage via ties, as a coverage table. No egress. The overview calls this "account mapping". See [tech-spec/playbooks.md](tech-spec/playbooks.md). |
| **age** | Small, modern file-encryption tool and format (X25519 keys). Used to encrypt view files to a recipient's public key. Keypair generated at `openstance init`, stored at `~/.openstance/age.key`. |
| **`allow_egress`** | Boolean parameter on every money-spending CLI command, MCP tool and HTTP endpoint. Defaults to `false` (from `enrich.allow_egress_default`). When false, the operation returns a **quote** instead of running. |
| **B, breadth** | Score component: `channels_seen / 4` over email, calendar, LinkedIn DM and social. |
| **bm25** | The ranking function FTS5 uses for full-text relevance. Normalized and fused with cosine similarity in search. |
| **`brief`** | Playbook and MCP tool. A 10-line markdown intro brief for one person: last interactions, ranked mutuals, optional web search for recent posts. |
| **Bulk sender** | A sender with 50+ messages and zero replies either way, or a `List-Unsubscribe` header on most messages. Marked `person.is_bulk = 1`, scored 0, excluded from search by default (`--include-bulk` to show). |
| **C, reciprocity** | Score component: `min(1, replies_from_them / max(1, messages_from_you))`; a thread they started counts double. |
| **Candidate set** | In search, the union of the top 200 FTS5 hits and the top 200 vector hits, filtered by the structured `Query` fields, before fusion. |
| **cargo-dist** | Release tool that builds cross-platform binaries, installers and a Homebrew formula from the Cargo workspace. Used in Phase 4. |
| **Commitment** | A promise the user recorded about a person ("send Priya the deck"), with a due date, snooze and done. Planned for Phase 5 (P5-MEM-01); a model-extracted commitment stays a suggestion until the user accepts it. Not built. |
| **Data directory** | `~/.openstance/`: `graph.db`, `config.toml`, `age.key`, cached models. |
| **`do_not_share`** | Per-person scalar override flag. The person never appears in exported views, and by default is never sent to an enricher. Always respected. |
| **E.164** | International phone-number format (`+15555550142`). Phones are normalized to it before identity matching. |
| **Egress** | Any operation that sends data about the user's contacts off the machine. In openstance the only egress path is an **Enricher** call (and, in Phase 5, sync of already-encrypted views). Off by default, quoted first, logged. |
| **Embedder** | Model trait that turns text into vectors. Default `fastembed` with `bge-small-en-v1.5` (384 dimensions). Changing it triggers a re-index. See [tech-spec/models.md](tech-spec/models.md). |
| **`enrich_log`** | Table with one row per enrichment request actually sent: what was sent, to which backend, for which playbook and person, quoted and actual cost, status. The audit trail for egress. |
| **Enricher** | Trait for fetching outside data (`Profile`, `FindEmail`, `VerifyEmail`, `Company`, `WebSearch`) with `quote()` before `enrich()`. Implementations: `Null` (default), `treg`, `OwnKeys`, `MockEnricher` (tests). See [tech-spec/enrichment.md](tech-spec/enrichment.md). |
| **Entity resolution** | Turning many identities from many sources into one `person` per human, using ordered, conservative rules with confidences. See [tech-spec/entity-resolution.md](tech-spec/entity-resolution.md). |
| **Evidence line** | A deterministic, checkable reason attached to a result: a dated interaction (thread subject, meeting title, who cc'd whom) or a matched field ("title contains founder"). Distinct from model-written prose, which is optional and labeled. |
| **Evidence class** | Where a fact or edge came from: `observed` (an email, meeting or DM in your data), `curated_export` (a LinkedIn connection, contact card or follow), `user_assertion` (your edits, merges, intros), `enrichment` (an `enrich:<backend>` result) or `model_interpretation` (model output). Only observed interactions or user assertions can establish a relationship between two other people. Proposed in [ADR-0014](decisions/0014-evidence-classes-and-claim-provenance.md); not yet on the JSON. |
| **Export-first** | Ingestion strategy: read files the user exported themselves (Takeout mbox, LinkedIn CSV, vcf, ics, X archive) rather than OAuth connectors. [ADR-0004](decisions/0004-export-first-ingestion.md). |
| **F, frequency** | Score component: `1 - exp(-n_interactions / 20)`. |
| **Federation** | Phase 5 team mode: each member keeps their own graph and exports views to a shared git or S3 remote; search fans out over imported views at query time. No central database. |
| **Flagged merge** | An automatic merge made with confidence below 1.0 but at or above 0.75 (name + company, name + mutual thread). Shown in `openstance review` for confirmation or split. |
| **FTS5** | SQLite's full-text search extension. `person_fts` is a regular (content-storing) FTS5 table with `rowid = person.id`. |
| **Fusion** | Combining text and vector relevance: bm25 and cosine each min-max normalized over the candidate set, summed 0.5/0.5, then multiplied by `(0.5 + 0.5 * score)`. |
| **Gmail aliasing** | Treating `a.b@gmail.com`, `ab@gmail.com` and `ab+x@gmail.com` as one identity (confidence 0.98). |
| **Half-life** | `score.half_life_days`, default 180. The number of days after which recency R falls to 0.5. |
| **`how_connected`** | MCP tool: top three paths from you to a person already in your graph, each with evidence. CLI equivalent: `openstance how <target>` (alias `how-connected`), see [tech-spec/interfaces.md](tech-spec/interfaces.md). |
| **Handoff** | An integration where openstance prepares something (for example a brief) and the user sends it through another service themselves. No API call is made and nothing is shown as "connected". Boardy is a handoff ([plan/phase-5-and-beyond.md](plan/phase-5-and-beyond.md), P5-RES-03). |
| **Identity** | A handle that points at a person: email, LinkedIn URL, X handle, phone, Instagram handle, or name+company. Stored normalized in `identity` with a confidence; `UNIQUE(kind, value)`. |
| **Importer** | Trait that turns a file into a stream of `RawIdentity` and `RawInteraction` records via a `Sink`. Has `kind()`, `detect()` and `import()`. Never writes `person`. See [tech-spec/ingestion.md](tech-spec/ingestion.md). |
| **Interaction** | One dated event between you and one or more people: email, meeting, DM, mention, follow or connection. The raw evidence for scores and ties. Deduplicated by `(source_id, external_id)`. |
| **`interaction_person`** | Join table linking an interaction to each participant with a role (`from`, `to`, `cc`, `attendee`, `sender`, `recipient`). Also the evidence store for ties. |
| **Jaro-Winkler** | String similarity measure. At 0.92 or above it only **suggests** a merge; it never auto-merges. |
| **Kill/park criterion** | If there is no meaningful Show HN response within a month of launch, the project is parked. |
| **M, meetings** | Score component: `1 - exp(-n_meetings / 3)`. |
| **MCP** | Model Context Protocol. `openstance mcp` runs a stdio MCP server (and `serve` exposes it over HTTP) with tools `search_people`, `get_person`, `how_connected`, `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich`. See [tech-spec/interfaces.md](tech-spec/interfaces.md). |
| **`max_usd`** | Per-call spending cap passed with `allow_egress`. Cannot exceed `enrich.max_usd_per_run` (default 0.50). |
| **Micro-USD** | Integer unit for money (1 USD = 1,000,000). Quotes, receipts and `enrich_log` never use floats for money. |
| **Mutual** | A person in your graph who has a tie to a target. Path strength through a mutual = `my_score(mutual) * tie_weight(mutual, target)`. |
| **`no_enrich`** | Per-person scalar override flag: the person is never sent to any enricher, not even by name. Always respected. |
| **Null implementation** | The do-nothing default for each model or enricher trait (`Null` reranker keeps fused order; `Null` enricher quotes 0 and returns nothing), so the binary works with nothing installed. |
| **`.ost` file** | An exported view: an age-encrypted SQLite file containing `person` (filtered), `identity` (emails and LinkedIn only) and `meta`. No interactions. |
| **`ost`** | Short alias installed for the `openstance` binary. |
| **Override** | A user edit stored in `override(id, person_id, field, value, set_at)` that survives rebuilds and is never overwritten by import or enrichment. **Scalar** fields (`display_name, title, company, location, bio, do_not_share, no_enrich`, plus `is_bulk` for correcting bulk detection) are last-write-wins by `set_at`; **multi-valued** fields (`tag, note, merged_into, split_from`) keep every row. |
| **`path` / `path_to`** | Playbook (CLI `openstance path`, MCP `path_to`) to reach a stranger: look up the target, optionally enrich their employers and school, intersect with your graph, rank mutuals, optionally find and verify an email. May spend money. |
| **Person** | One row per human after resolution; `person.id` is the anchor identity id (smallest identity id in the cluster), so ids are stable across rebuilds and merges, with display fields, `first_seen`, `last_contact`, cached `score`, `is_bulk`, `do_not_share`, `no_enrich`. Derived; rebuildable from sources plus overrides. |
| **Possible overlap** | A route to a target based only on an employer, past employer or school in common (often from enrichment). Not evidence that anyone knows anyone; shown apart from warm paths and never as a probability. See [ADR-0014](decisions/0014-evidence-classes-and-claim-provenance.md). |
| **`person_vec`** | sqlite-vec `vec0` virtual table holding one 384-dimension embedding per person. |
| **Phase** | A one-week (Phases 1 to 4) delivery increment. Phase 0 is the existing v0.1 prototype; Phase 5 is post-launch and gated on traction. See [plan/roadmap.md](plan/roadmap.md). |
| **Playbook** | A small deterministic workflow over the graph and an `Enricher` with a fixed step list: `path`, `accounts`, `refresh`, `watch`, `brief`, `reconnect`, `verify`. Has `plan()` (steps + quoted cost) and `run()`. Each ships as an openstance-authored markdown skill in `playbooks/`. See [tech-spec/playbooks.md](tech-spec/playbooks.md). |
| **Preset** | A named set of score weights: `founder`, `recruiter`, `seller`, `community`. Without a preset the built-in default weights are R .35, F .20, C .25, M .15, B .05. See [tech-spec/scoring.md](tech-spec/scoring.md). |
| **Query** | The structured search request (`text`, `company`, `title`, `location`, `industry`, `tags`, `since`, `until`, `last_contact_before`, `met_in_person`, `replied`, `min_score`, `lost_touch`, `include_bulk`, `limit`; see [tech-spec/search.md](tech-spec/search.md#2-the-query-struct)). Produced by a `QueryParser` or CLI flags; returned to the caller so filters can be edited and rerun. |
| **QueryParser** | Trait that turns free text into a `Query`. Default `RuleParser` (regex); optional model parsers whose JSON is validated against `Query`, falling back to rules on failure. |
| **Quote** | The priced plan of egress calls returned before anything is sent: each request, the exact outbound fields, and cost in micro-USD, plus a total. |
| **R, recency** | Score component: `exp(-ln(2) * days_since_last / half_life_days)`; 0.5 at 180 days by default. |
| **RawIdentity / RawInteraction** | Records an importer emits to a `Sink`. RawIdentity: kind, value, name hint, company hint, source ref. RawInteraction: kind, at, direction, subject, participants. |
| **Rebuild** | `openstance rebuild`: replay all sources, then all overrides, then recompute ties, scores and indexes. The graph is a pure function of sources plus overrides; rebuild is idempotent. |
| **Receipt** | The record returned after a playbook spends money: each call made, actual cost, total, and the `enrich_log` run id. |
| **`reconnect`** | Playbook: people with F > 0.5 and R < 0.2, sorted by past score, top 10 with the last subject. No egress. |
| **`refresh`** | Playbook: enrich people with empty title/company and 5+ interactions (not `no_enrich`), within budget. |
| **Reranker** | Optional trait to reorder the top 50 results (local cross-encoder or API model, which may also write a one-line reason). Default `Null`. |
| **`review`** | `openstance review`: list flagged merges and suggestions for bulk confirm or split. |
| **Score** | The relationship score `S = w_r R + w_f F + w_c C + w_m M + w_b B`, each component in [0, 1], all components stored and shown. [ADR-0005](decisions/0005-deterministic-explained-score.md). |
| **Score bucket** | The coarse score shown in an exported view instead of raw components : `strong` (score >= 0.60), `medium` (>= 0.30) or `weak`; thresholds `share.bucket_strong` / `share.bucket_medium`. |
| **Sink** | Trait importers write into (`identity()`, `interaction()`); owned by core. |
| **Source** | One imported file or enrichment backend: `source(id, kind, label, imported_at, file_hash)`. `kind` is an importer kind (`mbox`, `linkedin_csv`, `vcf`, `ics`, `x_archive`, ...) or `enrich:<backend>`. Same `file_hash` means re-import is a no-op. Raw source rows are never edited. |
| **sqlite-vec** | SQLite extension providing the `vec0` virtual table for brute-force vector similarity. |
| **Split** | `openstance split <person>`: writes a `split_from` override so two identities stay apart even if a later import matches exactly. |
| **Takeout** | Google's data export service; produces the Gmail `.mbox` file the `mbox` importer reads. |
| **Tie** | Derived person-to-person edge from shared interactions, of kind `email` (on the same message or thread), `meeting` (same event) or `dm` (same group DM). LinkedIn exports carry no mutual-connection data, so there is no LinkedIn tie. Row stores `kind, weight, last_at, n_shared`; evidence is `interaction_person` joined on both people. |
| **Tie weight** | `1 - exp(-D / tie_scale)`, where `D` sums the shared interactions with the same recency decay as R; in [0, 1]. Used for path strength. |
| **Tombstone** | An override row with `value = NULL` for a scalar field, meaning "remove my edit and fall back to derived data". Different from `''`, which means "empty on purpose". |
| **treg** | A third-party data router (treg.to) with one key for many data endpoints. One `Enricher` implementation among several; never required, never promoted. |
| **View** | A saved query (`view` table), the unit of sharing. Exported as an `.ost` file encrypted to a recipient. [ADR-0008](decisions/0008-views-as-encrypted-files.md), [tech-spec/sharing-and-views.md](tech-spec/sharing-and-views.md). |
| **`verify`** | Playbook: best email from identities, verify it, attach the freshest mutual who replied. One egress call. |
| **Warm path** | `you -> mutual -> target` where both hops rest on observed interactions or a user assertion (for the mutual-target hop, a tie from shared threads, meetings or group DMs). Returned by `how_connected` and `path` with evidence per hop. Contrast with **possible overlap**. |
| **`watch`** | Playbook: diff the latest two LinkedIn exports, optionally profile-check the top 100 by score, flag company changes. |
| **`who`** | CLI command: one person with score components, inputs and evidence (`get_person` in MCP). |

## Terms from the v0.1 prototype

These appear in [prototype-v0.1.md](prototype-v0.1.md) and may return in later phases.

| Term | Meaning in v0.1 | Status in the Rust design |
|---|---|---|
| **Matched / missing terms** | For each result, which query terms matched a structured field and which did not. | Kept as an idea: search responses expose which `Query` filters and text terms each result satisfied. |
| **Strict search** | Only results that match every term. | Structured filters are always hard constraints; free text is ranked. |
| **Intro stages** | Shortlisted, Asked for intro, Introduced, Meeting booked. | Built in Phase 4 as the `shortlist` table and the `intro` table with stages `draft, asked, intro_made, met, closed` ([ADR-0013](decisions/0013-react-vite-web-ui-and-workspace.md)); extension proposed in Q25. |
| **`via`** | A user-asserted connector name. | Replaced by derived ties and `how_connected` paths with evidence. |
| **Freshness warning** | Record older than 90 days flagged as stale. | Replaced by recency R and dated evidence lines. |
