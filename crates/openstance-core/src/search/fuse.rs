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
    /// Raw negated bm25 (`None` when not an FTS hit).
    pub fts_raw: Option<f64>,
    /// Raw cosine similarity (`None` when not a vector hit).
    pub vec_raw: Option<f64>,
    /// Fused relevance.
    pub relevance: f64,
    /// After the score lift.
    pub final_score: f64,
}

/// Min-max normalize `(id, raw)` pairs.
pub fn min_max(hits: &[(PersonId, f64)]) -> Vec<(PersonId, f64)> {
    let Some(min) = hits.iter().map(|h| h.1).reduce(f64::min) else {
        return Vec::new();
    };
    let max = hits.iter().map(|h| h.1).fold(min, f64::max);
    let span = max - min;
    hits.iter()
        .map(|&(id, x)| {
            let n = if span > 0.0 { (x - min) / span } else { 1.0 };
            (id, n)
        })
        .collect()
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
    use std::collections::BTreeMap;

    let fts_raw: BTreeMap<PersonId, f64> = fts.iter().copied().collect();
    let vec_raw: BTreeMap<PersonId, f64> = vector.unwrap_or(&[]).iter().copied().collect();
    let fts_n: BTreeMap<PersonId, f64> = min_max(fts).into_iter().collect();
    let vec_n: Option<BTreeMap<PersonId, f64>> = vector.map(|v| min_max(v).into_iter().collect());
    let mut ids: Vec<PersonId> = fts_n.keys().copied().collect();
    if let Some(v) = &vec_n {
        ids.extend(v.keys().copied());
    }
    ids.sort_unstable();
    ids.dedup();

    let both = !fts_n.is_empty() && vec_n.as_ref().is_some_and(|v| !v.is_empty());
    let mut out: Vec<(Fused, f64)> = ids
        .into_iter()
        .map(|id| {
            let f = fts_n.get(&id).copied().unwrap_or(0.0);
            let v = vec_n.as_ref().map(|m| m.get(&id).copied().unwrap_or(0.0));
            let relevance = match v {
                Some(v) if both => text_weight * f + (1.0 - text_weight) * v,
                Some(v) if fts_n.is_empty() => v,
                _ => f,
            };
            let score = score_of(id);
            let final_score = relevance * ((1.0 - relationship_lift) + relationship_lift * score);
            (
                Fused {
                    id,
                    fts: f,
                    vector: v,
                    fts_raw: fts_raw.get(&id).copied(),
                    vec_raw: vec_raw.get(&id).copied(),
                    relevance,
                    final_score,
                },
                score,
            )
        })
        .collect();
    out.sort_by(|(a, sa), (b, sb)| {
        b.final_score
            .total_cmp(&a.final_score)
            .then(sb.total_cmp(sa))
            .then(a.id.cmp(&b.id))
    });
    out.into_iter().map(|(f, _)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_single_candidate_in_each_list_does_not_divide_by_zero() {
        let out = fuse(&[(1, 3.0)], Some(&[(2, 0.4)]), 0.5, 0.5, &|_| 0.0);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .all(|f| f.relevance == 0.5 && f.final_score.is_finite()));
        let one = fuse(&[(1, 3.0)], Some(&[(1, 0.4)]), 0.5, 0.5, &|_| 1.0);
        assert_eq!(one[0].relevance, 1.0);
        assert_eq!(one[0].final_score, 1.0);
    }

    #[test]
    fn a_strong_relationship_lifts_a_weaker_text_match() {
        // A: best text match, a stranger. B: slightly weaker text, close.
        let fts = [(1, 10.0), (2, 9.0), (3, 2.0)];
        let vec = [(1, 0.90), (2, 0.88), (3, 0.40)];
        let score = |id: PersonId| match id {
            1 => 0.0,
            2 => 0.95,
            _ => 1.0,
        };
        let out = fuse(&fts, Some(&vec), 0.5, 0.5, &score);
        let order: Vec<_> = out.iter().map(|f| f.id).collect();
        assert_eq!(order, [2, 1, 3]);
        // ...but a cold exact match keeps half its relevance
        assert_eq!(out[1].final_score, 0.5);
    }

    #[test]
    fn raw_signals_are_kept_next_to_the_normalized_ones() {
        let out = fuse(
            &[(1, 9.0), (2, 6.0)],
            Some(&[(2, 0.7), (3, 0.6)]),
            0.5,
            0.5,
            &|_| 0.5,
        );
        let by = |id| *out.iter().find(|f| f.id == id).unwrap();
        assert_eq!((by(1).fts_raw, by(1).vec_raw), (Some(9.0), None));
        assert_eq!((by(2).fts_raw, by(2).vec_raw), (Some(6.0), Some(0.7)));
        assert_eq!((by(3).fts_raw, by(3).vec_raw), (None, Some(0.6)));
        let fts_only = fuse(&[(1, 9.0)], None, 0.5, 0.5, &|_| 0.5);
        assert_eq!(fts_only[0].vec_raw, None);
    }

    #[test]
    fn min_max_edge_cases() {
        assert!(min_max(&[]).is_empty());
        assert_eq!(min_max(&[(7, 3.2)]), [(7, 1.0)]);
        assert_eq!(min_max(&[(1, 2.0), (2, 2.0)]), [(1, 1.0), (2, 1.0)]);
        assert_eq!(
            min_max(&[(1, 9.0), (2, 6.0), (3, 3.0)]),
            [(1, 1.0), (2, 0.5), (3, 0.0)]
        );
    }

    /// The worked example of search.md section 6.3.
    #[test]
    fn worked_example_from_the_spec() {
        let fts = [(1, 9.0), (2, 6.0), (3, 3.0)];
        let vec = [(1, 0.82), (2, 0.78), (3, 0.70), (4, 0.80)];
        let score = |id: PersonId| match id {
            1 => 0.37,
            2 => 0.89,
            _ => 0.61,
        };
        let out = fuse(&fts, Some(&vec), 0.5, 0.5, &score);
        let order: Vec<_> = out.iter().map(|f| f.id).collect();
        assert_eq!(order, [1, 2, 4, 3]);
        let by = |id| out.iter().find(|f| f.id == id).unwrap();
        assert!(close(by(1).final_score, 0.685));
        assert!(close(by(2).relevance, 0.583));
        assert!(close(by(2).final_score, 0.551));
        assert!(close(by(4).relevance, 0.417));
        assert!(close(by(4).final_score, 0.335));
        assert_eq!(by(4).fts, 0.0);
        assert!(close(by(4).vector.unwrap(), 0.833));
        assert_eq!(by(3).final_score, 0.0);
    }

    #[test]
    fn fts_only_uses_fts_relevance_and_the_score_lift() {
        let fts = [(5, 4.0), (6, 2.0)];
        let out = fuse(&fts, None, 0.5, 0.5, &|id| if id == 6 { 1.0 } else { 0.0 });
        assert_eq!(out[0].id, 5);
        assert_eq!(out[0].vector, None);
        assert_eq!(out[0].relevance, 1.0);
        assert_eq!(out[0].final_score, 0.5, "score 0 keeps half the relevance");
        assert_eq!(out[1].final_score, 0.0);
    }

    #[test]
    fn empty_fts_uses_vector_relevance() {
        let out = fuse(&[], Some(&[(3, 0.9), (4, 0.5)]), 0.5, 0.5, &|_| 1.0);
        assert_eq!(out[0].id, 3);
        assert_eq!(out[0].relevance, 1.0);
        assert_eq!(out[1].relevance, 0.0);
    }

    #[test]
    fn ties_break_by_score_then_id() {
        let fts = [(9, 1.0), (3, 1.0), (5, 1.0)];
        let out = fuse(&fts, None, 0.5, 0.0, &|id| if id == 5 { 0.9 } else { 0.1 });
        let order: Vec<_> = out.iter().map(|f| f.id).collect();
        assert_eq!(order, [5, 3, 9]);
    }
}
