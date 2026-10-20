//! Tie derivation: person pairs sharing `email`/`meeting`/`dm` interactions
//! with 2..=`tie_max_participants` non-self, non-bulk participants.
//! Spec: `docs/tech-spec/scoring.md` section 8 and `data-model.md` 4.6
//! (P1-CORE-13).
//!
//! Owned by the "score" stream. `weight = 1 - exp(-D / tie_scale)` with
//! `D = sum(exp(-ln2 * age_days / half_life_days))`; `n_shared` is the plain
//! count; evidence is never stored.

use rusqlite::Connection;

use super::components::decay;
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
    let d: f64 = ages_days.iter().map(|a| decay(*a, half_life_days)).sum();
    weight_from_decayed(d, tie_scale)
}

/// `1 - exp(-D / tie_scale)`.
fn weight_from_decayed(d: f64, tie_scale: f64) -> f64 {
    1.0 - (-d / tie_scale).exp()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// scoring.md section 9.4.
    #[test]
    fn worked_path_example_weights() {
        let priya = tie_weight(&[20.0, 45.0, 90.0, 200.0, 400.0], 180.0, 3.0);
        assert!((priya - 0.6502).abs() < 1e-4, "{priya}");
        let dana = tie_weight(&[5.0], 180.0, 3.0);
        assert!((dana - 0.2789).abs() < 1e-4, "{dana}");
        let marcus = tie_weight(&[500.0, 600.0], 180.0, 3.0);
        assert!((marcus - 0.0784).abs() < 1e-4, "{marcus}");
    }

    #[test]
    fn tie_weight_edges() {
        assert_eq!(tie_weight(&[], 180.0, 3.0), 0.0);
        // Future interactions clamp to age 0 (decay term 1).
        let w = tie_weight(&[-10.0], 180.0, 3.0);
        assert!((w - (1.0 - (-1.0f64 / 3.0).exp())).abs() < 1e-12);
        assert!(tie_weight(&[0.0; 1000], 180.0, 3.0) < 1.0 + 1e-12);
    }
}
