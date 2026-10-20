//! Tie derivation: person pairs sharing `email`/`meeting`/`dm` interactions
//! with 2..=`tie_max_participants` non-self, non-bulk participants.
//! Spec: `docs/tech-spec/scoring.md` section 8 and `data-model.md` 4.6
//! (P1-CORE-13).
//!
//! Owned by the "score" stream. `weight = 1 - exp(-D / tie_scale)` with
//! `D = sum(exp(-ln2 * age_days / half_life_days))`; `n_shared` is the plain
//! count; evidence is never stored.

use rusqlite::Connection;

use super::ScoreConfig;
use crate::error::Result;
use crate::PersonId;

/// One shared interaction behind a tie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TieEvidence {
    /// `interaction.id`.
    pub interaction_id: i64,
    /// `interaction.kind`.
    pub kind: String,
    /// Unix seconds.
    pub at: i64,
    /// Subject.
    pub subject: Option<String>,
    /// `interaction.source_id`.
    pub source_id: i64,
}

/// `1 - exp(-D / tie_scale)` over the given ages (days).
pub fn tie_weight(ages_days: &[f64], half_life_days: f64, tie_scale: f64) -> f64 {
    let _ = (ages_days, half_life_days, tie_scale);
    todo!("score stream: tie_weight")
}

/// Delete and recompute ties for everyone (`people = None`) or for every
/// pair touching `people`. Returns tie rows written.
pub fn rebuild(
    conn: &Connection,
    cfg: &ScoreConfig,
    people: Option<&[PersonId]>,
    now: i64,
) -> Result<u64> {
    let _ = (conn, cfg, people, now);
    todo!("score stream: P1-CORE-13 ties rebuild")
}

/// The canonical evidence query: shared contact interactions of `a` and `b`,
/// newest first.
pub fn evidence(
    conn: &Connection,
    a: PersonId,
    b: PersonId,
    limit: usize,
) -> Result<Vec<TieEvidence>> {
    let _ = (conn, a, b, limit);
    todo!("score stream: tie evidence")
}
