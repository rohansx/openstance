//! `merge` and `split`: override writes with the anchor-swap and pinning
//! rules. Spec: `docs/tech-spec/entity-resolution.md` section 9 (P1-CLI-06
//! core side).
//!
//! Owned by the "resolve" stream. These functions only write overrides; the
//! caller runs incremental re-resolution afterwards.

use rusqlite::Connection;

use crate::error::Result;
use crate::{IdentityId, PersonId};

/// Result of [`merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// The surviving person (smallest anchor).
    pub person: PersonId,
    /// Override ids written.
    pub overrides: Vec<i64>,
    /// `split_from` rows deleted.
    pub split_rows_deleted: usize,
}

/// Result of [`split`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitOutcome {
    /// The side that kept the original anchor.
    pub kept: PersonId,
    /// The new person (anchor of the moved set).
    pub moved: PersonId,
    /// True when the sets were swapped to keep the anchor on the kept side.
    pub swapped: bool,
    /// Override ids written.
    pub overrides: Vec<i64>,
}

/// Merge `people` (at least two distinct persons) into the first.
pub fn merge(conn: &Connection, people: &[PersonId], now: i64) -> Result<MergeOutcome> {
    let _ = (conn, people, now);
    todo!("resolve stream: merge")
}

/// `merge --undo`: delete `merged_into` rows between the two clusters.
pub fn unmerge(conn: &Connection, a: PersonId, b: PersonId) -> Result<usize> {
    let _ = (conn, a, b);
    todo!("resolve stream: unmerge")
}

/// Move `identities` out of `person` into a new person, pinning both sides.
pub fn split(
    conn: &Connection,
    person: PersonId,
    identities: &[IdentityId],
    now: i64,
) -> Result<SplitOutcome> {
    let _ = (conn, person, identities, now);
    todo!("resolve stream: split")
}

/// `split --undo`: delete `split_from` rows between them (and the pins with
/// `unpin`).
pub fn unsplit(conn: &Connection, a: PersonId, b: PersonId, unpin: bool) -> Result<usize> {
    let _ = (conn, a, b, unpin);
    todo!("resolve stream: unsplit")
}
