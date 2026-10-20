//! The review queue: flagged merges, suggestions (rule 5, Jaro-Winkler,
//! large-company guard) and info rows, plus the decisions that write
//! overrides. Spec: `docs/tech-spec/entity-resolution.md` section 8
//! (P1-CLI-05 core side).
//!
//! Owned by the "resolve" stream. Decisions only write `override` rows; the
//! caller (`Graph`) then runs incremental re-resolution in the same
//! transaction.

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::json::ReviewQueue;
use crate::PersonId;

/// Filter for bulk confirmation (`review --confirm-all`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfirmFilter {
    /// Only this rule (`name_company` / `name_thread`).
    pub rule: Option<String>,
    /// Only flagged merges at or above this confidence.
    pub min_confidence: Option<f64>,
}

/// Build the queue, sorted by the higher score of the people involved.
pub fn queue(conn: &Connection, cfg: &Config) -> Result<ReviewQueue> {
    let _ = (conn, cfg);
    todo!("resolve stream: P1-CORE-07 review queue")
}

/// Confirm a flagged merge: `merged_into` pins each flagged identity to the
/// anchor. Returns the override ids written.
pub fn confirm(conn: &Connection, person: PersonId, now: i64) -> Result<Vec<i64>> {
    let _ = (conn, person, now);
    todo!("resolve stream: review confirm")
}

/// Confirm every flagged merge matching `filter`. Returns how many people.
pub fn confirm_all(
    conn: &Connection,
    cfg: &Config,
    filter: &ConfirmFilter,
    now: i64,
) -> Result<usize> {
    let _ = (conn, cfg, filter, now);
    todo!("resolve stream: review confirm_all")
}

/// Reject a suggestion: `split_from` between the two anchors.
pub fn reject(conn: &Connection, a: PersonId, b: PersonId, now: i64) -> Result<i64> {
    let _ = (conn, a, b, now);
    todo!("resolve stream: review reject")
}
