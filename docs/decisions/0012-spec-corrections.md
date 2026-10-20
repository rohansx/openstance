# ADR-0012: Corrections to the original tech spec

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/data-model.md](../tech-spec/data-model.md), [tech-spec/scoring.md](../tech-spec/scoring.md), [tech-spec/entity-resolution.md](../tech-spec/entity-resolution.md), [tech-spec/search.md](../tech-spec/search.md), [tech-spec/configuration.md](../tech-spec/configuration.md)

## Context

The original "openstance tech spec" PDF (17 pages, 2026-10-20) and the "openstance product overview" PDF (13 pages) contain internal inconsistencies and a few statements that would not work as written. The docs in this repository present the **corrected** design. This ADR records every correction so a reader comparing the docs with the PDFs knows which is authoritative: **the docs are**.

## Decision: schema and algorithm corrections (a to g)

### a. Table count and `enrich_log`

| Original PDF | Problem | Resolved |
|---|---|---|
| "Eight tables": `source, person, identity, interaction, interaction_person, tie, override, view` | The enrichment section requires an `enrich_log` table that is never defined | **Nine regular tables**: `source, person, identity, interaction, interaction_person, tie, override, view, enrich_log`, plus virtual tables `person_fts` (FTS5) and `person_vec` (vec0), plus a `schema_version` migrations table |

`enrich_log` has one row per request actually **sent** (unapproved quotes are not logged, since nothing left the machine). Minimum content this ADR requires: run id, timestamp, backend, playbook, `person_id`, operation, the exact outbound `request_json`, quoted and actual cost in integer micro-USD, status, and the `enrich:<backend>` `source_id` the result was stored under. The full DDL and indexes are owned by [tech-spec/data-model.md](../tech-spec/data-model.md); the write path is in [tech-spec/enrichment.md](../tech-spec/enrichment.md).

### b. Bulk senders

| Original | Problem | Resolved |
|---|---|---|
| Bulk senders "get `kind='bulk'` and score 0" | `person` has no `kind` column; `interaction.kind` is the wrong level | `person.is_bulk INTEGER NOT NULL DEFAULT 0`. Bulk people get score 0 and are **excluded from search by default** (`--include-bulk` to show them) |

### c. Tie evidence

| Original | Problem | Resolved |
|---|---|---|
| Overview: "every tie points at the source rows that created it" | `tie` has no evidence column or join table | Ties are derived from shared interactions. Evidence for a tie between A and B = `interaction_person` joined on both people (no extra table). The `tie` row stores `kind, weight, last_at, n_shared` |

```sql
CREATE TABLE tie (
  a INTEGER NOT NULL REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  b INTEGER NOT NULL REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  kind TEXT NOT NULL CHECK (kind IN ('email','meeting','dm')),  -- no linkedin_mutual: exports carry no mutuals
  weight REAL NOT NULL,       -- 1 - exp(-D / tie_scale), see tech-spec/scoring.md
  last_at INTEGER NOT NULL,
  n_shared INTEGER NOT NULL,
  PRIMARY KEY (a, b, kind),
  CHECK (a < b)
);
-- evidence for a tie (a, b):
SELECT i.* FROM interaction i
JOIN interaction_person pa ON pa.interaction_id = i.id AND pa.person_id = :a
JOIN interaction_person pb ON pb.interaction_id = i.id AND pb.person_id = :b
ORDER BY i.at DESC;
```

The `CHECK (a < b)` canonical ordering is a recommendation of this ADR so each pair is stored once; [tech-spec/data-model.md](../tech-spec/data-model.md) is authoritative on the final DDL.

### d. Recency formula

| Original | Problem | Resolved |
|---|---|---|
| `R = exp(-days_since_last / half_life)`, "half-life 180 days" | With that formula R = e^-1 = 0.37 at 180 days, so 180 is a time constant, not a half-life | `R = exp(-ln(2) * days_since_last / half_life_days)`, `half_life_days = 180` default, so R = 0.5 at 180 days |

### e. Override table

| Original | Problem | Resolved |
|---|---|---|
| `PRIMARY KEY (person_id, field, value)` | A scalar field (title) can hold several rows with no rule for which wins; NULL in a PK column; removing an edit is undefined | `override(id INTEGER PRIMARY KEY, person_id, field, value, set_at)`. Scalar fields (`display_name, title, company, location, bio, do_not_share, no_enrich`) are **last-write-wins by `set_at`**. Multi-valued fields (`tag, note, merged_into, split_from`) keep every row. Removal: a scalar tombstone row with `value = NULL`; for multi-valued fields, delete the row |

### f. FTS5 table type

| Original | Problem | Resolved |
|---|---|---|
| `person_fts USING fts5(..., content='')` (contentless) | Contentless FTS5 tables cannot return column values (no snippets/highlight for evidence) and do not support ordinary `DELETE`/update of changed people | `person_fts` is a **regular, content-storing** FTS5 table keyed by `rowid = person.id`. On any person change: delete + insert that rowid. `openstance rebuild` fully rebuilds it |

### g. Default score weights

| Original | Problem | Resolved |
|---|---|---|
| "Default weights [0.35, 0.20, 0.25, 0.15, 0.05]" and a `founder` preset of [0.30, 0.15, 0.30, 0.20, 0.05], with the sample config setting `preset = "founder"` | Two different "defaults" | Built-in default weights (R, F, C, M, B) = **0.35, 0.20, 0.25, 0.15, 0.05** (founder-leaning), used when no preset or weights are configured. Named presets `founder`, `recruiter`, `seller`, `community` keep the spec's values. See [tech-spec/scoring.md](../tech-spec/scoring.md) |

## Decision: other reconciliations

| Topic | Original | Resolved |
|---|---|---|
| Product name | Overview recommends "kith"; spec and prototype use "openstance"; spec page footers say "kith tech spec" | Working name "openstance" everywhere; rename is open in [ADR-0001](0001-product-name.md) |
| MCP "who-knows" tool | Overview lists `who-knows` as an MCP tool; spec does not | No separate tool: `search_people` with a company filter + `how_connected` ([ADR-0011](0011-one-json-shape-across-surfaces.md)) |
| MVP length | Overview "about two weeks"; spec four weekly milestones | Phase 1 + Phase 2 (two weeks) = public MVP and Show HN; Phases 3 and 4 complete v1 ([plan/roadmap.md](../plan/roadmap.md)) |
| MVP content | Overview MVP includes `path` via treg and `share` | `path`, Enricher/treg and views move to Phase 3; the Phase 2 demo shows import, search with evidence, `how_connected`, and MCP from Claude Code |
| Happenstance free tier | Overview: "about 15 searches a month free" | Unconfirmed third-party figure; do not state it as fact. Pricing per their page: $24/mo billed annually, $30 monthly ([product/competitive-analysis.md](../product/competitive-analysis.md)) |
| `source.kind` values | Enumerated list omits enrichment sources | Also allows `enrich:<backend>` (for example `enrich:treg`) |
| Playbook list | Overview mentions "account mapping"; spec has `path, accounts, refresh, watch, brief, reconnect, verify` | Spec list is canonical; "account mapping" = `accounts` |

## Consequences

- Implementers follow the docs, not the PDFs. Any further discrepancy found is fixed in the docs and appended here.
- Migrations 0001 in Phase 1 create the corrected schema directly; there is no legacy schema to migrate.

## Alternatives considered

- **Leave the PDFs authoritative and footnote issues:** rejected; implementers would build the bugs.
- **Edit the PDFs:** they are source documents from 2026-10-20 and stay unchanged for history.
