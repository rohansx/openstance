//! Rebuild: make every derived table a pure function of raw rows, overrides
//! and config. Spec: `docs/tech-spec/entity-resolution.md` section 5 and
//! `data-model.md` section 1 (P1-CORE-09).
//!
//! Owned by the "resolve" stream. The caller owns the transaction. The
//! cross-module calls are fixed:
//!
//! ```text
//! rebuild(conn, cfg, now):
//!   res = resolve::resolve(conn, cfg)                     // steps 1-13
//!   m   = resolve::materialize_all(conn, &res)            // step 14
//!   resolve::fields::materialize_fields(conn, None)       // step 15
//!   score::recompute_all(conn, &score_cfg, now)           // step 16: bulk, scores, ties
//!   search::fts::reindex_all(conn)                        // step 17
//!   meta.last_rebuild_at = now                            // step 18
//!
//! incremental(conn, cfg, touched, now):                   // after import / merge / split
//!   res = resolve::resolve(conn, cfg)
//!   m   = resolve::materialize_diff(conn, &res, touched)
//!   resolve::fields::materialize_fields(conn, Some(&m.affected))
//!   score::recompute_people(conn, &score_cfg, &m.affected, now)
//!   search::fts::remove_people(conn, &m.removed); search::fts::reindex_people(conn, &m.affected)
//!
//! refresh_people(conn, cfg, people, now):                 // after edit (no re-clustering)
//!   resolve::fields::materialize_fields(conn, Some(people))
//!   score::recompute_people(conn, &score_cfg, people, now)
//!   search::fts::reindex_people(conn, people)
//! ```

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::error::Result;
use crate::ingest::Touched;
use crate::PersonId;

/// What a rebuild (full or incremental) did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RebuildReport {
    /// Person rows after the run (including self).
    pub people: u64,
    /// People recomputed.
    pub affected: u64,
    /// Person ids removed (absorbed by merges).
    pub removed: u64,
    /// People with flagged identities.
    pub flagged: u64,
    /// Bulk senders.
    pub bulk: u64,
    /// Tie rows.
    pub ties: u64,
    /// Unions blocked by `split_from`.
    pub blocked_by_override: u64,
    /// Conflicting must-link override ids (reported by `rebuild` / `doctor`).
    pub conflicting_overrides: Vec<i64>,
    /// Wall time.
    pub took_ms: u64,
}

/// Full rebuild of all derived data (steps 4-18).
pub fn rebuild(conn: &Connection, cfg: &Config, now: i64) -> Result<RebuildReport> {
    let _ = (conn, cfg, now);
    todo!("resolve stream: P1-CORE-09 rebuild")
}

/// Rebuild-lite after an import, merge or split. Must give the same derived
/// tables as [`rebuild`] would.
pub fn incremental(
    conn: &Connection,
    cfg: &Config,
    touched: &Touched,
    now: i64,
) -> Result<RebuildReport> {
    let _ = (conn, cfg, touched, now);
    todo!("resolve stream: P1-CORE-09 incremental")
}

/// Steps 15-17 for people whose clusters did not change (after `edit`).
pub fn refresh_people(
    conn: &Connection,
    cfg: &Config,
    people: &[PersonId],
    now: i64,
) -> Result<()> {
    let _ = (conn, cfg, people, now);
    todo!("resolve stream: P1-CORE-09 refresh_people")
}
