# ADR-0014: Evidence classes: every fact and edge says where it came from

- **Status:** Proposed
- **Date:** 2026-10-21
- **Related:** [ADR-0005](0005-deterministic-explained-score.md), [ADR-0007](0007-enricher-trait-egress-off.md), [ADR-0009](0009-conservative-entity-resolution.md), [ADR-0011](0011-one-json-shape-across-surfaces.md), [tech-spec/data-model.md](../tech-spec/data-model.md), [tech-spec/search.md](../tech-spec/search.md#8-step-6-evidence), [tech-spec/scoring.md](../tech-spec/scoring.md#8-ties-and-path-strength), [product/github-inspiration.md](../product/github-inspiration.md)

## Context

openstance already keeps provenance, but in several places with no shared name:

- `source.kind` says which importer or backend produced raw rows (`mbox`, `linkedin_csv`, `vcf`, `ics`, `x_archive`, `enrich:<backend>`).
- `interaction.kind` says what happened (`email`, `meeting`, `dm`, `connection`, `contact`, `follow`, `mention`). [scoring.md](../tech-spec/scoring.md#21-which-interactions-count) already treats `follow` and `mention` as weak (breadth only) and `contact` rows as not counting at all.
- `override` rows are the user's own edits, merges and splits; `shortlist` and `intro` (schema v4, [ADR-0013](0013-react-vite-web-ui-and-workspace.md)) are user-authored too.
- Enrichment facts live in `enrich_log.response_json` and are replayed under an `enrich:<backend>` source; they show up as `Evidence::Enrich` items with the `enrich_log` id.
- Model prose is labelled `reason_source: "model:<id>"`; hosted model calls are logged in `enrich_log` as `model:<provider>` ([models.md](../tech-spec/models.md#9-privacy-and-egress-rules-for-models)).

What is missing is a rule about **which kinds of evidence may establish a relationship**. The `path` playbook shows why it matters: for a target outside the graph it intersects the target's employers and school (often from enrichment) with the user's graph and ranks the result by `my_score x affinity`, with fixed affinities 1.0, 0.6 and 0.4 (`crates/openstance-core/src/playbook/path.rs`). That is a sensible heuristic, but the output shares the `strength` field with tie-based paths built from real shared threads and meetings. A reader can take "worked at the same company" for "knows the target". Open-source references point the same way: Relationship Substrate types its sources (direct interaction, curated export, enrichment, derived interpretation), and Lean Intros' overlap score is a heuristic, not a probability (see [github-inspiration.md](../product/github-inspiration.md)).

## Decision

### 1. Five evidence classes

Every fact on a person and every edge between people has exactly one class:

| Class | Meaning | Comes from today |
|---|---|---|
| `observed` | An interaction the user's own data shows happened: a message sent or received, a meeting attended, a DM exchanged | `interaction.kind` in `email`, `meeting`, `dm` from `mbox`, `ics`, `x_archive` DMs and LinkedIn `messages.csv`; ties derived from those (`tie.kind` `email`, `meeting`, `dm`). `mention` is observed but weak, as scoring already says |
| `curated_export` | A list a platform or address book kept for the user: a LinkedIn connection, a contact card, an X follow | `interaction.kind` `connection` (LinkedIn `Connections.csv`, invitations), `contact` (vcf, and csv when built), `follow` (X archive); importer hints such as LinkedIn Position and vCard ORG |
| `user_assertion` | Something the user stated in openstance | `override` rows (fields, `tag`, `note`, `merged_into`, `split_from`, privacy flags), review confirmations (which are overrides), `shortlist`, `intro` |
| `enrichment` | Data an `Enricher` fetched from outside, after a quote | `source.kind = 'enrich:<backend>'`, `enrich_log` rows with `status = 'ok'`, enrichment identities (`match_rule = 'enrich'`), `Evidence::Enrich` |
| `model_interpretation` | Something a model inferred | `reason_source: "model:<id>"` on reranked results, a model `QueryParser`'s extracted filters (`query_source`), the `semantic` matched entry (embedding similarity), and any future model-extracted suggestion |

The class is **derived at read time** from columns that already exist (`source.kind`, `interaction.kind`, the table a row lives in, `reason_source`). No migration is needed.

### 2. What each class may establish

- **A relationship between the user and a person** may be established by `observed` interactions, by `user_assertion`, or by a `curated_export` of the user's own first-degree contacts. The last one is shown as what it is ("in your LinkedIn connections since 2024-03", "in your contacts"), never as "you have talked". This keeps current scoring, where a LinkedIn connection counts as one contact event ([scoring.md](../tech-spec/scoring.md#21-which-interactions-count)); whether it should is Q26.
- **A relationship between two other people** (a tie, a hop on a warm path) may be established **only** by `observed` interactions or a `user_assertion`. This is already true for ties (no LinkedIn tie exists, [data-model.md](../tech-spec/data-model.md#46-tie-derived)); the decision makes it a rule.
- **`enrichment` and public web results** may support profile facts (title, company, past employers, school, email) and **possible overlaps** (shared employer or school). They never establish a relationship, between anyone. An article or company page can support a profile fact; it cannot establish a private relationship.
- **`model_interpretation`** never establishes a fact or a relationship. It can rank, summarise or suggest. A suggestion becomes a fact only when the user accepts it, and then it is a `user_assertion`.

### 3. Paths carry a basis

A path is either `observed` (the mutual has a tie with the target, or the user asserted the link) or `possible_overlap` (employer, past employer or school in common). Proposed JSON changes, additive within the current `v`:

- each `Evidence` item gains `"class": "observed" | "curated_export" | "user_assertion" | "enrichment" | "model_interpretation"`;
- each `PathJson` gains `"basis": "observed" | "possible_overlap"`;
- when no path exists, the response says so with a reason (`"no_path": "no_mutual_ties" | "target_not_in_graph_and_no_enricher" | ...`) instead of an empty array alone.

The shapes are owned by [interfaces.md](../tech-spec/interfaces.md); the fields are added there when this ADR is accepted (tasks P4-PATH-01, P4-PATH-02, P4X-CORE-01 in the [roadmap](../plan/roadmap.md#research-backlog-2026-10-21)).

### 4. Presentation rules

- A `possible_overlap` path is never labelled "warm", never shown as a percentage or probability, and is listed after observed paths, under its own heading ("Possible overlaps: not known relationships").
- `strength` stays a ranking number. Surfaces do not present it as a likelihood of an introduction.
- The evidence for every hop shows date, source label, class, and whose relationship it is (yours with the mutual, the mutual's with the target). Missing evidence is shown as missing, not hidden.

## Consequences

**Positive**

- One vocabulary across the data model, JSON, UI and agent skills; agents can filter by class without guessing from `source.kind`.
- Enrichment, Exa or treg results cannot quietly turn into "warm paths". The provider backlog (P5-RES-*) inherits the rule.
- No migration and no change to scoring or resolution.

**Negative**

- The JSON grows by two fields and changes are needed in interfaces.md, playbooks.md, the UI and the MCP snapshot tests.
- The `curated_export` exception for first-degree contacts is a judgement call: a LinkedIn connection does not mean the user knows the person well. It is labelled honestly and tracked as Q26.
- Read-time derivation means the class mapping table must be kept in step with every new importer and interaction kind; a test should fail when a new `source.kind` or `interaction.kind` has no class.

## Alternatives considered

| Option | Why not |
|---|---|
| Store a `class` column on `interaction` and `identity` | Duplicates information already implied by `source.kind` and `interaction.kind`; a migration for no new information |
| Fold possible overlaps into tie strength with a lower weight | Hides the difference the user most needs to see; turns a heuristic into a number that looks like evidence |
| Drop employer/school intersection from `path` | Useful for targets outside the graph when it is labelled as a possible overlap; the user can still ask the mutual |
| Let enrichment establish ties (for example "both worked at X 2019 to 2021") | Co-employment is not acquaintance; this is the misleading heuristic the rule exists to prevent |
