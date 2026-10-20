//! Entity resolution: identities -> people. Conservative by design: a wrong
//! merge is worse than a missed one. Spec: `docs/tech-spec/entity-resolution.md`.
//!
//! Owned by the "resolve" stream (P1-CORE-06, -07, -12 and the review /
//! merge / split commands). Module map:
//! - [`normalize`]: storage normalization and match keys (section 2);
//! - [`rules`]: data files, common names, rule confidences (section 3);
//! - [`clusters`]: union-find, rules 1-4, dormant filter (steps 1-13);
//! - [`fields`]: person fields from hints + overrides (step 15);
//! - [`review`]: the review queue and its decisions (section 8);
//! - [`commands`]: merge / split (section 9);
//! - [`lookup`]: person references (id, email, slug, name).

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::ingest::Touched;
use crate::{IdentityId, PersonId};

pub mod clusters;
pub mod commands;
pub mod fields;
pub mod lookup;
pub mod normalize;
pub mod review;
pub mod rules;

/// Which rule attached an identity to its person (`identity.match_rule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchRule {
    /// The cluster's anchor (minimum identity id).
    Anchor,
    /// Rule 1: same handle or source-asserted co-reference.
    Exact,
    /// Rule 2: Gmail / plus aliasing.
    GmailAlias,
    /// Rule 3: name + company (flagged).
    NameCompany,
    /// Rule 4: name + mutual thread (flagged).
    NameThread,
    /// A `merged_into` override or `[me]` must-link.
    Override,
    /// An enrichment identity (Phase 3).
    Enrich,
}

impl MatchRule {
    /// The string stored in `identity.match_rule`.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchRule::Anchor => "anchor",
            MatchRule::Exact => "exact",
            MatchRule::GmailAlias => "gmail_alias",
            MatchRule::NameCompany => "name_company",
            MatchRule::NameThread => "name_thread",
            MatchRule::Override => "override",
            MatchRule::Enrich => "enrich",
        }
    }

    /// Parse a stored `match_rule`.
    pub fn parse(s: &str) -> Option<MatchRule> {
        [
            MatchRule::Anchor,
            MatchRule::Exact,
            MatchRule::GmailAlias,
            MatchRule::NameCompany,
            MatchRule::NameThread,
            MatchRule::Override,
            MatchRule::Enrich,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }

    /// Fixed confidence of the rule (enrich is `<= 0.9`, reported per edge).
    pub fn confidence(self) -> f64 {
        match self {
            MatchRule::Anchor | MatchRule::Exact | MatchRule::Override => 1.0,
            MatchRule::GmailAlias => 0.98,
            MatchRule::NameCompany => 0.85,
            MatchRule::NameThread => 0.75,
            MatchRule::Enrich => 0.9,
        }
    }

    /// Merged but listed in `review` until confirmed or split.
    pub fn is_flagged(self) -> bool {
        matches!(self, MatchRule::NameCompany | MatchRule::NameThread)
    }
}

/// Where one identity landed after resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Assignment {
    /// The person (anchor = minimum identity id of the cluster).
    pub person_id: PersonId,
    /// Min confidence over the union-tree path to the anchor.
    pub confidence: f64,
    /// Rule of the weakest edge on that path.
    pub rule: MatchRule,
}

/// The in-memory result of steps 1-13: a pure function of raw rows,
/// overrides, config and data files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    /// Every identity that belongs to a person. Dormant identities are absent.
    pub assignments: BTreeMap<IdentityId, Assignment>,
    /// The person containing a `[me]` identity, if any.
    pub self_person: Option<PersonId>,
    /// Unions skipped because of a `split_from`.
    pub blocked_by_override: u64,
    /// `override.id`s of must-links skipped as conflicting.
    pub conflicting_overrides: Vec<i64>,
}

impl Resolution {
    /// Distinct person ids, ascending.
    pub fn people(&self) -> Vec<PersonId> {
        let mut v: Vec<PersonId> = self.assignments.values().map(|a| a.person_id).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// What changed when a [`Resolution`] was written to the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Materialized {
    /// People whose identity set or interactions changed (fields, score,
    /// ties and FTS must be recomputed for these).
    pub affected: Vec<PersonId>,
    /// Person ids that no longer exist (their anchors were absorbed).
    pub removed: Vec<PersonId>,
}

/// Steps 1-13 of the rebuild algorithm, in memory, over all identities.
pub fn resolve(conn: &Connection, cfg: &Config) -> Result<Resolution> {
    clusters::resolve(conn, cfg)
}

/// Step 14, full: clear derived assignments and write `person` rows,
/// `identity.person_id/confidence/match_rule`, `interaction_person.person_id`,
/// `person.is_self` and `interaction.direction` for everyone.
pub fn materialize_all(conn: &Connection, res: &Resolution) -> Result<Materialized> {
    let before: BTreeSet<PersonId> = stored_people(conn)?;
    conn.execute_batch(
        "UPDATE identity SET person_id = NULL, confidence = NULL, match_rule = NULL;
         UPDATE interaction_person SET person_id = NULL;
         DELETE FROM tie;
         DELETE FROM person_fts;
         DELETE FROM person;",
    )?;
    let people = res.people();
    {
        let mut ins = conn.prepare_cached("INSERT INTO person(id, is_self) VALUES (?1, ?2)")?;
        for &p in &people {
            ins.execute(rusqlite::params![p, res.self_person == Some(p)])?;
        }
        let mut upd = conn.prepare_cached(
            "UPDATE identity SET person_id = ?2, confidence = ?3, match_rule = ?4 WHERE id = ?1",
        )?;
        for (id, a) in &res.assignments {
            upd.execute(rusqlite::params![
                id,
                a.person_id,
                a.confidence,
                a.rule.as_str()
            ])?;
        }
    }
    conn.execute_batch(
        "UPDATE interaction_person SET person_id =
           (SELECT i.person_id FROM identity i WHERE i.id = interaction_person.identity_id);",
    )?;
    write_self_cache(conn, res)?;
    recompute_direction(conn, None)?;
    let now: BTreeSet<PersonId> = people.iter().copied().collect();
    Ok(Materialized {
        affected: people,
        removed: before.difference(&now).copied().collect(),
    })
}

/// Step 14, incremental: diff `res` against the stored assignment and write
/// only what changed. People touched by `touched` count as affected even if
/// their identity set is unchanged. Must leave the database exactly as
/// [`materialize_all`] would (incremental equals full).
pub fn materialize_diff(
    conn: &Connection,
    res: &Resolution,
    touched: &Touched,
) -> Result<Materialized> {
    let stored = stored_assignments(conn)?;
    let before_people = stored_people(conn)?;
    let before_self: BTreeSet<PersonId> = conn
        .prepare("SELECT id FROM person WHERE is_self = 1")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let now_people: BTreeSet<PersonId> = res.people().into_iter().collect();
    let mut affected: BTreeSet<PersonId> = BTreeSet::new();

    let removed: Vec<PersonId> = before_people.difference(&now_people).copied().collect();
    // Identities whose assignment changed (including ones that became dormant).
    let mut changed: Vec<IdentityId> = Vec::new();
    for (id, old) in &stored {
        let new = res.assignments.get(id);
        let same = match (old, new) {
            (None, None) => true,
            (Some((p, c, r)), Some(a)) => {
                *p == a.person_id && *c == a.confidence && r == a.rule.as_str()
            }
            _ => false,
        };
        if !same {
            changed.push(*id);
            if let Some((p, _, _)) = old {
                if now_people.contains(p) {
                    affected.insert(*p);
                }
            }
            if let Some(a) = new {
                affected.insert(a.person_id);
            }
        }
    }
    for id in &touched.identities {
        if let Some(a) = res.assignments.get(id) {
            affected.insert(a.person_id);
        }
    }
    for p in &touched.people {
        // A touched person id is an anchor: follow it to its current person.
        match res.assignments.get(p) {
            Some(a) => {
                affected.insert(a.person_id);
            }
            None if now_people.contains(p) => {
                affected.insert(*p);
            }
            None => {}
        }
    }

    {
        let mut ins = conn.prepare_cached("INSERT INTO person(id) VALUES (?1)")?;
        for p in now_people.difference(&before_people) {
            ins.execute([p])?;
            affected.insert(*p);
        }
        let mut upd = conn.prepare_cached(
            "UPDATE identity SET person_id = ?2, confidence = ?3, match_rule = ?4 WHERE id = ?1",
        )?;
        let mut ip = conn.prepare_cached(
            "UPDATE interaction_person SET person_id = ?2 WHERE identity_id = ?1",
        )?;
        for id in &changed {
            match res.assignments.get(id) {
                Some(a) => {
                    upd.execute(rusqlite::params![
                        id,
                        a.person_id,
                        a.confidence,
                        a.rule.as_str()
                    ])?;
                    ip.execute(rusqlite::params![id, a.person_id])?;
                }
                None => {
                    upd.execute(rusqlite::params![
                        id,
                        None::<i64>,
                        None::<f64>,
                        None::<String>
                    ])?;
                    ip.execute(rusqlite::params![id, None::<i64>])?;
                }
            }
        }
        // New interaction rows of touched identities carry no person yet.
        for id in &touched.identities {
            let p = res.assignments.get(id).map(|a| a.person_id);
            ip.execute(rusqlite::params![id, p])?;
        }
    }
    // Removed people last, once nothing references them.
    if !removed.is_empty() {
        let mut del_tie = conn.prepare_cached("DELETE FROM tie WHERE a = ?1 OR b = ?1")?;
        let mut del_person = conn.prepare_cached("DELETE FROM person WHERE id = ?1")?;
        for p in &removed {
            del_tie.execute([p])?;
            del_person.execute([p])?;
        }
    }
    let now_self: BTreeSet<PersonId> = res.self_person.into_iter().collect();
    let self_changed = now_self != before_self;
    if self_changed {
        conn.execute("UPDATE person SET is_self = 0 WHERE is_self = 1", [])?;
        if let Some(s) = res.self_person {
            conn.execute("UPDATE person SET is_self = 1 WHERE id = ?1", [s])?;
        }
        affected.extend(before_self.intersection(&now_people).copied());
        affected.extend(now_self.iter().copied());
    }
    write_self_cache(conn, res)?;
    if self_changed {
        // The self person changed: every direction may change.
        recompute_direction(conn, None)?;
    } else {
        let ids: BTreeSet<IdentityId> = changed
            .iter()
            .chain(touched.identities.iter())
            .copied()
            .collect();
        recompute_direction(conn, Some(&ids))?;
    }
    affected.retain(|p| now_people.contains(p));
    Ok(Materialized {
        affected: affected.into_iter().collect(),
        removed,
    })
}

fn stored_people(conn: &Connection) -> Result<BTreeSet<PersonId>> {
    Ok(conn
        .prepare("SELECT id FROM person")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// identity id -> stored (person, confidence, rule), `None` when unassigned.
#[allow(clippy::type_complexity)]
fn stored_assignments(
    conn: &Connection,
) -> Result<BTreeMap<IdentityId, Option<(PersonId, f64, String)>>> {
    let mut st = conn.prepare("SELECT id, person_id, confidence, match_rule FROM identity")?;
    let rows = st.query_map([], |r| {
        let p: Option<i64> = r.get(1)?;
        let c: Option<f64> = r.get(2)?;
        let m: Option<String> = r.get(3)?;
        Ok((
            r.get::<_, i64>(0)?,
            p.map(|p| (p, c.unwrap_or(0.0), m.unwrap_or_default())),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `meta.self_identity_ids`: the identities of the self person, as JSON.
fn write_self_cache(conn: &Connection, res: &Resolution) -> Result<()> {
    let ids: Vec<IdentityId> = match res.self_person {
        Some(s) => res
            .assignments
            .iter()
            .filter(|(_, a)| a.person_id == s)
            .map(|(id, _)| *id)
            .collect(),
        None => Vec::new(),
    };
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('self_identity_ids', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [serde_json::to_string(&ids)?],
    )?;
    Ok(())
}

const DIRECTION_SQL: &str = "UPDATE interaction SET direction = CASE
    WHEN EXISTS (SELECT 1 FROM interaction_person ip JOIN person p ON p.id = ip.person_id
                 WHERE ip.interaction_id = interaction.id AND p.is_self = 1
                   AND ip.role IN ('from', 'sender', 'organizer')) THEN 'out'
    WHEN EXISTS (SELECT 1 FROM interaction_person ip JOIN person p ON p.id = ip.person_id
                 WHERE ip.interaction_id = interaction.id AND p.is_self = 1)
      THEN CASE kind WHEN 'meeting' THEN 'mutual' ELSE 'in' END
    ELSE COALESCE(dir_hint, 'none') END";

/// Recompute `interaction.direction` (data-model.md 4.3) for every
/// interaction, or only those where one of `identities` takes part.
pub fn recompute_direction(
    conn: &Connection,
    identities: Option<&BTreeSet<IdentityId>>,
) -> Result<()> {
    match identities {
        None => {
            conn.execute(DIRECTION_SQL, [])?;
        }
        Some(ids) => {
            let mut st = conn.prepare_cached(&format!(
                "{DIRECTION_SQL} WHERE id IN (SELECT interaction_id FROM interaction_person WHERE identity_id = ?1)"
            ))?;
            for id in ids {
                st.execute([id])?;
            }
        }
    }
    Ok(())
}
