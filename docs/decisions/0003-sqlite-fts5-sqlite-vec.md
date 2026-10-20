# ADR-0003: One SQLite file with FTS5 and sqlite-vec, not ClickHouse

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/data-model.md](../tech-spec/data-model.md), [tech-spec/search.md](../tech-spec/search.md), [engineering/performance-targets.md](../engineering/performance-targets.md)

## Context

All user data (sources, people, identities, interactions, ties, overrides, views, enrichment log) plus the search indexes must live somewhere. ClickHouse was raised as an option. The workload and size were sized in the tech spec for a heavy user:

| Entity | Count |
|---|---|
| people | ~20k |
| identities | ~50k |
| interactions (every email header, meeting, DM) | ~500k |
| ties | ~100k |
| embedding vectors (384 dims, f32) | ~20k (about 30 MB) |
| **Total file** | **200 to 500 MB** |

The query mix is point lookups, small joins, full-text matches and frequent single-row edits (overrides, merges, notes). The largest scan is a brute-force cosine over about 20k vectors.

## Decision

Store everything in **one SQLite file per user** (`~/.openstance/graph.db`), using:

- `rusqlite` with bundled SQLite, `PRAGMA journal_mode=WAL; synchronous=NORMAL; foreign_keys=ON`.
- **FTS5** for full-text search over people (`person_fts`, a regular content-storing table; see [ADR-0012](0012-spec-corrections.md) item f).
- **sqlite-vec** (`vec0` virtual table `person_vec`, `float[384]`) for vector similarity, brute force.
- A `meta` table (schema version, migration history) and forward-only migrations embedded in the binary.

The schema is designed to export cleanly to Parquet (`openstance export --format parquet`) so a columnar engine stays possible for a future team tier.

## Consequences

**Positive**

- "No daemon": the database is a library call. Backup, sync and sharing are file copies.
- Point lookups in microseconds; FTS5 over 500k rows in low milliseconds; sqlite-vec over 20k vectors under 10 ms. These support the targets: structured search under 50 ms, text search with embedder under 300 ms.
- Cheap row updates for overrides and merges.
- Encryption at rest is available via an optional SQLCipher build flag; views are age-encrypted SQLite files using the same schema subset ([ADR-0008](0008-views-as-encrypted-files.md)).

**Negative**

- Single writer. Acceptable: imports run in one transaction; `serve` and `mcp` mostly read. Long imports block writes from the UI; the API returns `409 busy` rather than queueing.
- sqlite-vec is young; brute force does not scale past roughly 200k vectors. Acceptable at the target size; revisit if a user exceeds 100k people.
- Shipping sqlite-vec statically for all targets must be verified in CI (Phase 1).

## Alternatives considered

| Option | Why not |
|---|---|
| **ClickHouse** (server or clickhouse-local) | It is a server (or a ~500 MB binary) and breaks the "file on your machine" promise. OLAP engine: built for billion-row scans, weak point lookups, no cheap row updates. No FTS5 equivalent that fits a laptop; no vector index that beats brute force at 20k rows; no encryption-at-rest story for a shareable file. Backup and sharing become infrastructure. |
| DuckDB | Good analytics and Parquet, but weaker at frequent small transactional writes and no mature FTS/vector story equal to FTS5 + sqlite-vec. Kept as the second option for team analytics over exported Parquet. |
| Postgres + pgvector | Requires a server. First choice for a future cloud team tier with materialized views, not for local. |
| Tantivy + a separate KV store | Two stores to keep consistent; FTS5 is sufficient. |
| LanceDB / Qdrant embedded | Unnecessary at 20k vectors; adds a second storage engine. |

Where a columnar store could ever earn its place: a 1,000-seat cloud team that wants cross-member analytics. Order of preference then: Postgres with materialized views, DuckDB over exported Parquet, ClickHouse only for a customer with 100M+ interactions. See [product/pricing-and-cloud.md](../product/pricing-and-cloud.md).
