//! `how_connected` / `openstance how` (P2-CORE-05): warm paths
//! `me -> mutual -> target` inside the graph.
//!
//! Spec: `docs/tech-spec/scoring.md` 8.2 and 8.3, `interfaces.md` 1.7.
//!
//! - Candidates are the people with a tie to the target, never the owner and
//!   never bulk senders.
//! - Ties of several kinds combine as independent signals:
//!   `tie_strength = 1 - prod_k (1 - weight_k)`.
//! - `strength = my_score(mutual) * tie_strength(mutual, target)`; paths with
//!   strength 0 are dropped; the top `limit` (default 3) are returned,
//!   strongest first (ties broken by the mutual's score, then id).
//! - Each path's `evidence` is, in this order: one `tie` item per tie kind
//!   (strongest first), up to [`SHARED_EVIDENCE`] most recent interactions
//!   the mutual and the target share (joined through `interaction_person`,
//!   never stored), and last the user's own most recent interaction with the
//!   mutual.
//! - `direct` is set when the target has a last contact (a direct
//!   relationship always beats an intro).
//! - A target that is not in the graph is not an error: the response is
//!   [`NotInGraph`] and nothing leaves the machine.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::json::{
    iso, iso_opt, round3, Direct, Evidence, HowConnected, PathJson, SourceRefJson, TieWith, V,
};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::evidence::{line, recent};
use crate::search::person::person_ref;
use crate::PersonId;

/// Shown when the target is not in the graph.
pub const NOT_IN_GRAPH_HINT: &str = "path_to (Phase 3) can look outside your graph";

/// Shared interactions listed per path.
pub const SHARED_EVIDENCE: usize = 3;

/// Upper bound of `limit` (the MCP `how_connected` schema).
pub const MAX_PATHS: usize = 10;

/// Options for [`how_connected`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HowOptions {
    /// Paths to return, clamped to `1..=MAX_PATHS`.
    pub limit: usize,
}

impl Default for HowOptions {
    fn default() -> Self {
        HowOptions { limit: 3 }
    }
}

/// The target is not in the graph: `{ "v": 1, "found": false, "hint": ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotInGraph {
    /// Schema version.
    pub v: u32,
    /// Always `false`.
    pub found: bool,
    /// What to try instead.
    pub hint: String,
}

impl Default for NotInGraph {
    fn default() -> Self {
        NotInGraph {
            v: V,
            found: false,
            hint: NOT_IN_GRAPH_HINT.to_string(),
        }
    }
}

/// The result of [`how_connected`]: the `HowConnected` shape, or
/// [`NotInGraph`]. Serialized untagged, so the JSON is exactly one of the two.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HowConnectedResponse {
    /// The target is in the graph.
    Found(HowConnected),
    /// The target is not in the graph.
    NotFound(NotInGraph),
}

/// Warm paths to `target`. Ambiguous names are `Error::Ambiguous`; the owner
/// is `Error::BadRequest`; an unknown target is
/// [`HowConnectedResponse::NotFound`].
pub fn how_connected(
    conn: &Connection,
    target: &PersonKey,
    opts: HowOptions,
    now: i64,
) -> Result<HowConnectedResponse> {
    let target = match resolve_person(conn, target) {
        Ok(r) => r.person,
        Err(Error::NotFound(_)) => {
            return Ok(HowConnectedResponse::NotFound(NotInGraph::default()))
        }
        Err(e) => return Err(e),
    };
    let Some(t) = load_target(conn, target)? else {
        return Ok(HowConnectedResponse::NotFound(NotInGraph::default()));
    };
    if t.is_self {
        return Err(Error::BadRequest(
            "that is you: how_connected looks for paths to other people".into(),
        ));
    }
    let limit = opts.limit.clamp(1, MAX_PATHS);
    let mut paths = Vec::new();
    for m in mutuals(conn, target)?.into_iter().take(limit) {
        paths.push(path(conn, &t, m, now)?);
    }
    let target_ref = person_ref(conn, target)?
        .ok_or_else(|| Error::Internal(format!("person {target} vanished")))?;
    Ok(HowConnectedResponse::Found(HowConnected {
        v: V,
        direct: t.last_contact.map(|at| Direct {
            score: round3(t.score),
            last_contact: iso_opt(Some(at)),
        }),
        target: target_ref,
        paths,
    }))
}

struct Target {
    id: PersonId,
    display_name: Option<String>,
    score: f64,
    last_contact: Option<i64>,
    is_self: bool,
}

fn load_target(conn: &Connection, id: PersonId) -> Result<Option<Target>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, display_name, score, last_contact, is_self FROM person WHERE id = ?1",
        )?
        .query_row([id], |r| {
            Ok(Target {
                id: r.get(0)?,
                display_name: r.get(1)?,
                score: r.get(2)?,
                last_contact: r.get(3)?,
                is_self: r.get::<_, i64>(4)? != 0,
            })
        })
        .optional()?)
}

/// One tie row between a mutual and the target.
struct TieRow {
    kind: String,
    weight: f64,
    n_shared: u32,
    last_at: i64,
}

/// A candidate mutual with all its tie kinds to the target.
struct Mutual {
    id: PersonId,
    score: f64,
    ties: Vec<TieRow>,
    tie_strength: f64,
    strength: f64,
}

/// Every eligible mutual of `target`, strongest path first.
fn mutuals(conn: &Connection, target: PersonId) -> Result<Vec<Mutual>> {
    // `tie(a, ..)` is the primary key prefix and `tie_b` covers `b`, so the
    // two halves of the UNION are both index lookups.
    let mut st = conn.prepare_cached(
        "SELECT t.m, t.kind, t.weight, t.n_shared, t.last_at, p.score
         FROM (SELECT b AS m, kind, weight, n_shared, last_at FROM tie WHERE a = ?1
               UNION ALL
               SELECT a AS m, kind, weight, n_shared, last_at FROM tie WHERE b = ?1) t
         JOIN person p ON p.id = t.m
         WHERE p.is_self = 0 AND p.is_bulk = 0 AND p.score > 0 AND t.m <> ?1
         ORDER BY t.m, t.weight DESC, t.kind",
    )?;
    let rows = st.query_map([target], |r| {
        Ok((
            r.get::<_, PersonId>(0)?,
            TieRow {
                kind: r.get(1)?,
                weight: r.get(2)?,
                n_shared: r.get(3)?,
                last_at: r.get(4)?,
            },
            r.get::<_, f64>(5)?,
        ))
    })?;
    let mut out: Vec<Mutual> = Vec::new();
    for row in rows {
        let (m, tie, score) = row?;
        match out.last_mut() {
            Some(last) if last.id == m => last.ties.push(tie),
            _ => out.push(Mutual {
                id: m,
                score,
                ties: vec![tie],
                tie_strength: 0.0,
                strength: 0.0,
            }),
        }
    }
    for m in &mut out {
        let miss: f64 = m
            .ties
            .iter()
            .map(|t| 1.0 - t.weight.clamp(0.0, 1.0))
            .product();
        m.tie_strength = 1.0 - miss;
        m.strength = m.score * m.tie_strength;
    }
    out.retain(|m| m.strength > 0.0);
    out.sort_by(|x, y| {
        y.strength
            .total_cmp(&x.strength)
            .then(y.score.total_cmp(&x.score))
            .then(x.id.cmp(&y.id))
    });
    Ok(out)
}

fn path(conn: &Connection, t: &Target, m: Mutual, now: i64) -> Result<PathJson> {
    let via = person_ref(conn, m.id)?
        .ok_or_else(|| Error::Internal(format!("person {} vanished", m.id)))?;
    let mut evidence: Vec<Evidence> = m
        .ties
        .iter()
        .map(|tie| Evidence::Tie {
            with: TieWith {
                id: t.id,
                display_name: t.display_name.clone(),
            },
            tie_kind: tie.kind.clone(),
            n_shared: tie.n_shared,
            weight: round3(tie.weight),
            last_at: iso(tie.last_at),
        })
        .collect();
    evidence.extend(shared(conn, m.id, t.id, SHARED_EVIDENCE, now)?);
    if let Some(mut mine) = recent(conn, &[m.id], 1, now)?.remove(&m.id) {
        evidence.append(&mut mine);
    }
    Ok(PathJson {
        via,
        strength: round3(m.strength),
        reason: "tie".to_string(),
        my_score: round3(m.score),
        link_weight: round3(m.tie_strength),
        evidence,
    })
}

/// The `n` most recent `email/meeting/dm` interactions (at or before `now`)
/// that `via` and `target` are both on, as interaction evidence carrying the
/// target's role.
fn shared(
    conn: &Connection,
    via: PersonId,
    target: PersonId,
    n: usize,
    now: i64,
) -> Result<Vec<Evidence>> {
    let mut st = conn.prepare_cached(
        "SELECT i.id, i.kind, i.direction, i.at, i.subject, min(y.role),
                s.id, s.kind, s.label
         FROM interaction_person x
         JOIN interaction_person y ON y.interaction_id = x.interaction_id AND y.person_id = ?2
         CROSS JOIN interaction i ON i.id = x.interaction_id
         JOIN source s ON s.id = i.source_id
         WHERE x.person_id = ?1
           AND i.kind IN ('email', 'meeting', 'dm')
           AND i.at <= ?3
         GROUP BY i.id
         ORDER BY i.at DESC, i.id DESC
         LIMIT ?4",
    )?;
    let limit = i64::try_from(n).unwrap_or(i64::MAX);
    let rows = st.query_map(params![via, target, now, limit], |r| {
        let at: i64 = r.get(3)?;
        let kind: String = r.get(1)?;
        let direction: Option<String> = r.get(2)?;
        let subject: Option<String> = r.get(4)?;
        let label: Option<String> = r.get(8)?;
        let l = line(
            at,
            &kind,
            direction.as_deref(),
            subject.as_deref(),
            label.as_deref(),
        );
        Ok(Evidence::Interaction {
            interaction_id: r.get(0)?,
            kind,
            direction,
            at: iso(at),
            subject,
            role: r.get(5)?,
            source: SourceRefJson {
                id: r.get(6)?,
                kind: r.get(7)?,
                label,
            },
            line: l,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit::{ago, graph, NOW};
    use openstance_import::HandleKind;

    fn found(r: HowConnectedResponse) -> HowConnected {
        match r {
            HowConnectedResponse::Found(h) => h,
            HowConnectedResponse::NotFound(n) => panic!("not found: {n:?}"),
        }
    }

    fn how(c: &Connection, target: i64) -> HowConnected {
        found(how_connected(c, &PersonKey::Id(target), HowOptions::default(), NOW).unwrap())
    }

    fn vias(h: &HowConnected) -> Vec<i64> {
        h.paths.iter().map(|p| p.via.id).collect()
    }

    /// scoring.md 9.4: Sam (here Ravi, id 7) is reached through Priya,
    /// Dana and Marcus, in that order.
    #[test]
    fn worked_path_example_orders_by_score_times_tie_weight() {
        let c = graph();
        c.execute_batch(
            "UPDATE person SET score = 0.8853 WHERE id = 2;  -- Priya
             UPDATE person SET score = 0.6066 WHERE id = 3;  -- Marcus
             UPDATE person SET score = 0.3683, display_name = 'Dana Whitfield' WHERE id = 5;
             INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (2, 7, 'email', 0.6502, 0, 5),
               (5, 7, 'meeting', 0.2789, 0, 1),
               (3, 7, 'meeting', 0.0784, 0, 2);",
        )
        .unwrap();
        let h = how(&c, 7);
        assert_eq!(vias(&h), [2, 5, 3]);
        let s: Vec<f64> = h.paths.iter().map(|p| p.strength).collect();
        assert_eq!(s, [0.576, 0.103, 0.048]);
        let p = &h.paths[0];
        assert_eq!(p.reason, "tie");
        assert_eq!(p.my_score, 0.885);
        assert_eq!(p.link_weight, 0.65);
        assert_eq!(h.target.id, 7);
        assert_eq!(h.target.display_name.as_deref(), Some("Ravi Iyer"));
    }

    #[test]
    fn tie_kinds_combine_as_independent_signals() {
        let c = graph();
        c.execute_batch(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (3, 7, 'meeting', 0.5, 10, 1),
               (3, 7, 'email', 0.2, 20, 2),
               (2, 7, 'email', 0.3, 0, 1);",
        )
        .unwrap();
        let h = how(&c, 7);
        // Marcus: 1 - 0.5 * 0.8 = 0.6 -> 0.5 * 0.6 = 0.30; Priya: 0.8 * 0.3 = 0.24.
        assert_eq!(vias(&h), [3, 2]);
        assert_eq!(h.paths[0].link_weight, 0.6);
        assert_eq!(h.paths[0].strength, 0.3);
        let tie_kinds: Vec<&str> = h.paths[0]
            .evidence
            .iter()
            .filter_map(|e| match e {
                Evidence::Tie { tie_kind, .. } => Some(tie_kind.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tie_kinds, ["meeting", "email"], "strongest kind first");
    }

    #[test]
    fn owner_bulk_and_zero_score_people_are_never_mutuals() {
        let c = graph();
        c.execute_batch(
            "UPDATE person SET score = 0 WHERE id = 6;
             INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (1, 7, 'email', 0.9, 0, 9),
               (4, 7, 'email', 0.9, 0, 9),
               (6, 7, 'email', 0.9, 0, 9),
               (5, 7, 'email', 0.1, 0, 1);",
        )
        .unwrap();
        assert_eq!(vias(&how(&c, 7)), [5]);
    }

    #[test]
    fn limit_defaults_to_three_and_is_clamped() {
        let c = graph();
        c.execute_batch(
            "UPDATE person SET score = 0.3 WHERE id = 6;
             INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (2, 7, 'email', 0.5, 0, 1), (3, 7, 'email', 0.5, 0, 1),
               (5, 7, 'email', 0.5, 0, 1), (6, 7, 'email', 0.5, 0, 1);",
        )
        .unwrap();
        assert_eq!(vias(&how(&c, 7)), [2, 3, 5]);
        let all = |limit| {
            found(how_connected(&c, &PersonKey::Id(7), HowOptions { limit }, NOW).unwrap())
                .paths
                .len()
        };
        assert_eq!(all(0), 1);
        assert_eq!(all(1000), 4);
    }

    #[test]
    fn evidence_is_tie_then_shared_interactions_then_my_last_with_the_mutual() {
        let c = graph();
        // Priya (2) and Marcus (3) are both on 103 "Board deck draft".
        c.execute(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (2, 3, 'email', 0.4, ?1, 1)",
            [ago(60)],
        )
        .unwrap();
        let h = how(&c, 3);
        assert_eq!(vias(&h), [2]);
        let ev = &h.paths[0].evidence;
        assert_eq!(ev.len(), 3, "{ev:#?}");
        match &ev[0] {
            Evidence::Tie {
                with,
                tie_kind,
                n_shared,
                weight,
                last_at,
            } => {
                assert_eq!(with.id, 3);
                assert_eq!(with.display_name.as_deref(), Some("Marcus Lee"));
                assert_eq!(tie_kind, "email");
                assert_eq!(*n_shared, 1);
                assert_eq!(*weight, 0.4);
                assert_eq!(last_at, &iso(ago(60)));
            }
            other => panic!("{other:?}"),
        }
        match &ev[1] {
            Evidence::Interaction {
                interaction_id,
                subject,
                role,
                line,
                ..
            } => {
                assert_eq!(*interaction_id, 103);
                assert_eq!(subject.as_deref(), Some("Board deck draft"));
                assert_eq!(role.as_deref(), Some("cc"), "the target's role");
                assert!(line.contains("\"Board deck draft\""), "{line}");
            }
            other => panic!("{other:?}"),
        }
        match &ev[2] {
            Evidence::Interaction { interaction_id, .. } => {
                assert_eq!(*interaction_id, 101, "my own last interaction with Priya")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn shared_evidence_is_newest_first_and_capped() {
        let c = graph();
        for (i, days) in [(201, 50), (202, 10), (203, 30), (204, 20), (205, 1)] {
            c.execute(
                "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
                 VALUES (?1, 1, 'email', ?2, ?3, ?4, 'in')",
                params![i, ago(days), format!("s{i}"), format!("x{i}")],
            )
            .unwrap();
            for ident in [2, 7] {
                c.execute(
                    "INSERT INTO interaction_person(interaction_id, identity_id, role, person_id)
                     VALUES (?1, ?2, 'to', ?2)",
                    params![i, ident],
                )
                .unwrap();
            }
        }
        // A future meeting is not evidence yet.
        c.execute_batch(
            "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
               VALUES (206, 3, 'meeting', 1999999999, 'future', 'x206', 'mutual');
             INSERT INTO interaction_person(interaction_id, identity_id, role, person_id)
               VALUES (206, 2, 'attendee', 2), (206, 7, 'attendee', 7);
             INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (2, 7, 'email', 0.7, 0, 5);",
        )
        .unwrap();
        let h = how(&c, 7);
        let shared: Vec<i64> = h.paths[0].evidence[1..1 + SHARED_EVIDENCE]
            .iter()
            .map(|e| match e {
                Evidence::Interaction { interaction_id, .. } => *interaction_id,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(shared, [205, 202, 204]);
        assert_eq!(h.paths[0].evidence.len(), 1 + SHARED_EVIDENCE + 1);
    }

    #[test]
    fn direct_is_set_from_the_targets_score_and_last_contact() {
        let c = graph();
        let h = how(&c, 3);
        assert_eq!(
            h.direct,
            Some(Direct {
                score: 0.5,
                last_contact: Some(iso(ago(20)))
            })
        );
        assert!(h.paths.is_empty());
        // Lena (6) has no contact: no direct relationship.
        assert_eq!(how(&c, 6).direct, None);
    }

    #[test]
    fn target_resolves_by_handle_or_name() {
        let c = graph();
        let by_email = found(
            how_connected(
                &c,
                &PersonKey::Handle(HandleKind::Email, "marcus@contoso.example".into()),
                HowOptions::default(),
                NOW,
            )
            .unwrap(),
        );
        assert_eq!(by_email.target.id, 3);
        let by_name = found(
            how_connected(
                &c,
                &PersonKey::Name("marcus lee".into()),
                HowOptions::default(),
                NOW,
            )
            .unwrap(),
        );
        assert_eq!(by_name, by_email);
    }

    #[test]
    fn unknown_target_is_found_false_with_the_path_to_hint() {
        let c = graph();
        for key in [
            PersonKey::Id(999),
            PersonKey::Name("Kai Wu".into()),
            PersonKey::Handle(HandleKind::Email, "kai@ramp.example".into()),
        ] {
            let r = how_connected(&c, &key, HowOptions::default(), NOW).unwrap();
            assert_eq!(r, HowConnectedResponse::NotFound(NotInGraph::default()));
            assert_eq!(
                serde_json::to_string(&r).unwrap(),
                r#"{"v":1,"found":false,"hint":"path_to (Phase 3) can look outside your graph"}"#
            );
        }
    }

    #[test]
    fn the_owner_is_not_a_target_and_ambiguity_is_an_error() {
        let c = graph();
        let e = how_connected(&c, &PersonKey::Id(1), HowOptions::default(), NOW).unwrap_err();
        assert!(matches!(e, Error::BadRequest(_)), "{e:?}");
        c.execute(
            "INSERT INTO person(id, display_name) VALUES (60, 'Marcus Lee')",
            [],
        )
        .unwrap();
        let e = how_connected(
            &c,
            &PersonKey::Name("Marcus Lee".into()),
            HowOptions::default(),
            NOW,
        )
        .unwrap_err();
        assert!(matches!(e, Error::Ambiguous { .. }));
    }

    #[test]
    fn json_matches_the_interfaces_shape() {
        let c = graph();
        c.execute(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (2, 3, 'email', 0.4, 0, 1)",
            [],
        )
        .unwrap();
        let v = serde_json::to_value(how(&c, 3)).unwrap();
        let keys = |v: &serde_json::Value| -> Vec<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        assert_eq!(keys(&v), ["v", "target", "direct", "paths"]);
        assert_eq!(
            keys(&v["paths"][0]),
            [
                "via",
                "strength",
                "reason",
                "my_score",
                "link_weight",
                "evidence"
            ]
        );
        assert_eq!(v["paths"][0]["evidence"][0]["kind"], "tie");
        assert_eq!(v["paths"][0]["evidence"][1]["kind"], "interaction");
        assert_eq!(v["direct"]["last_contact"], iso(ago(20)));
    }
}
