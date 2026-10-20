//! Fusion arithmetic (step 4). Spec: `docs/tech-spec/search.md` section 6.3.
//!
//! Owned by the "search" stream. Min-max over each list (a single hit or
//! all-equal list gets 1.0), `relevance = w * fts_n + (1 - w) * vec_n` (or the
//! available half), `final = relevance * ((1 - lift) + lift * score)`.

use crate::PersonId;

/// One fused candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fused {
    /// Person.
    pub id: PersonId,
    /// Normalized FTS relevance.
    pub fts: f64,
    /// Normalized vector relevance (`None` without a vector half).
    pub vector: Option<f64>,
    /// Fused relevance.
    pub relevance: f64,
    /// After the score lift.
    pub final_score: f64,
}

/// Min-max normalize `(id, raw)` pairs.
pub fn min_max(hits: &[(PersonId, f64)]) -> Vec<(PersonId, f64)> {
    let _ = hits;
    todo!("search stream: min_max")
}

/// Fuse FTS and vector hits with the score lift; sorted by `final_score`
/// desc, then score desc, then id asc. `score_of` returns `person.score`.
pub fn fuse(
    fts: &[(PersonId, f64)],
    vector: Option<&[(PersonId, f64)]>,
    text_weight: f64,
    relationship_lift: f64,
    score_of: &dyn Fn(PersonId) -> f64,
) -> Vec<Fused> {
    let _ = (fts, vector, text_weight, relationship_lift, score_of);
    todo!("search stream: fuse")
}
