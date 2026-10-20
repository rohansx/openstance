//! Tie derivation: person pairs sharing `email`/`meeting`/`dm` interactions
//! with 2..=`tie_max_participants` non-self, non-bulk participants.
//! Spec: `docs/tech-spec/scoring.md` section 8 and `data-model.md` 4.6
//! (P1-CORE-13).
//!
//! Owned by the "score" stream. `weight = 1 - exp(-D / tie_scale)` with
//! `D = sum(exp(-ln2 * age_days / half_life_days))`; `n_shared` is the plain
//! count; evidence is never stored.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};

use super::components::decay;
use super::load::json_ids;
use super::ScoreConfig;
use crate::error::Result;
use crate::PersonId;

const DAY: f64 = 86_400.0;

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
///
/// A scoped rebuild widens `people` to everyone who shares an
/// email/meeting/dm with them: a bulk or merge change for one person can
/// move another interaction across the participant cap, which changes pairs
/// that do not include that person. Every pair touching the widened set is
/// recomputed from all of its shared interactions, so the result equals a
/// full rebuild. Rows pointing at people who no longer exist (absorbed by a
/// merge) are always removed.
pub fn rebuild(
    conn: &Connection,
    cfg: &ScoreConfig,
    people: Option<&[PersonId]>,
    now: i64,
) -> Result<u64> {
    conn.execute(
        "DELETE FROM tie WHERE a NOT IN (SELECT id FROM person) OR b NOT IN (SELECT id FROM person)",
        [],
    )?;
    let scope: Option<BTreeSet<PersonId>> = match people {
        None => {
            conn.execute("DELETE FROM tie", [])?;
            None
        }
        Some(p) => {
            let mut set: BTreeSet<PersonId> = p.iter().copied().collect();
            let mut st = conn.prepare(
                "SELECT DISTINCT y.person_id
                 FROM interaction_person x
                 JOIN interaction i ON i.id = x.interaction_id
                 JOIN interaction_person y ON y.interaction_id = x.interaction_id
                 WHERE x.person_id IN (SELECT value FROM json_each(?1))
                   AND i.kind IN ('email','meeting','dm')
                   AND y.person_id IS NOT NULL",
            )?;
            let rows = st.query_map([json_ids(p)], |r| r.get::<_, i64>(0))?;
            for r in rows {
                set.insert(r?);
            }
            let ids: Vec<PersonId> = set.iter().copied().collect();
            conn.execute(
                "DELETE FROM tie
                 WHERE a IN (SELECT value FROM json_each(?1))
                    OR b IN (SELECT value FROM json_each(?1))",
                [json_ids(&ids)],
            )?;
            Some(set)
        }
    };

    let scope_json = scope
        .as_ref()
        .map(|s| json_ids(&s.iter().copied().collect::<Vec<_>>()));
    let scope_filter = if scope_json.is_some() {
        "AND i.id IN (SELECT interaction_id FROM interaction_person
                      WHERE person_id IN (SELECT value FROM json_each(?2)))"
    } else {
        "AND ?2 IS NULL"
    };
    let sql = format!(
        "SELECT i.id, i.kind, i.at, ip.person_id
         FROM interaction i
         JOIN interaction_person ip ON ip.interaction_id = i.id
         JOIN person p ON p.id = ip.person_id
         WHERE i.kind IN ('email','meeting','dm')
           AND NOT (i.kind = 'meeting' AND i.at > ?1)
           AND p.is_self = 0 AND p.is_bulk = 0
           {scope_filter}
         ORDER BY i.id, ip.person_id"
    );
    let mut st = conn.prepare(&sql)?;
    let rows = st.query_map(params![now, scope_json], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;

    // (a, b, kind) -> (D, n_shared, last_at). BTreeMap keeps the insert
    // order (and so the result) deterministic.
    let mut acc: BTreeMap<(PersonId, PersonId, String), (f64, i64, i64)> = BTreeMap::new();
    let mut flush = |iid: Option<(i64, String, i64)>, members: &mut Vec<PersonId>| {
        if let Some((_, kind, at)) = iid {
            let n = members.len();
            if (2..=cfg.tie_max_participants).contains(&n) {
                let age = (now - at) as f64 / DAY;
                let d = decay(age, cfg.half_life_days);
                for (x, a) in members.iter().enumerate() {
                    for b in &members[x + 1..] {
                        let in_scope = scope
                            .as_ref()
                            .is_none_or(|s| s.contains(a) || s.contains(b));
                        if !in_scope {
                            continue;
                        }
                        let e = acc.entry((*a, *b, kind.clone())).or_insert((0.0, 0, at));
                        e.0 += d;
                        e.1 += 1;
                        e.2 = e.2.max(at);
                    }
                }
            }
        }
        members.clear();
    };

    let mut cur: Option<(i64, String, i64)> = None;
    let mut members: Vec<PersonId> = Vec::new();
    for row in rows {
        let (iid, kind, at, person) = row?;
        if cur.as_ref().map(|c| c.0) != Some(iid) {
            flush(cur.take(), &mut members);
            cur = Some((iid, kind, at));
        }
        // Rows are ordered by person id, so duplicates are adjacent.
        if members.last() != Some(&person) {
            members.push(person);
        }
    }
    flush(cur.take(), &mut members);

    let mut ins = conn.prepare(
        "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for ((a, b, kind), (d, n, last)) in &acc {
        ins.execute(params![
            a,
            b,
            kind,
            weight_from_decayed(*d, cfg.tie_scale),
            last,
            n
        ])?;
    }
    Ok(acc.len() as u64)
}

/// The canonical evidence query: shared contact interactions of `a` and `b`,
/// newest first.
pub fn evidence(
    conn: &Connection,
    a: PersonId,
    b: PersonId,
    limit: usize,
) -> Result<Vec<TieEvidence>> {
    if a == b {
        return Ok(Vec::new());
    }
    let mut st = conn.prepare_cached(
        "SELECT i.id, i.kind, i.at, i.subject, i.source_id
         FROM interaction_person x
         JOIN interaction_person y ON y.interaction_id = x.interaction_id AND y.person_id = ?2
         JOIN interaction i        ON i.id = x.interaction_id
         WHERE x.person_id = ?1
           AND i.kind IN ('email','meeting','dm')
         GROUP BY i.id
         ORDER BY i.at DESC, i.id DESC
         LIMIT ?3",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = st.query_map(params![a, b, limit], |r| {
        Ok(TieEvidence {
            interaction_id: r.get(0)?,
            kind: r.get(1)?,
            at: r.get(2)?,
            subject: r.get(3)?,
            source_id: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
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
