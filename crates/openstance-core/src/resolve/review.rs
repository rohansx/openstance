//! The review queue: flagged merges, suggestions (rule 5, Jaro-Winkler,
//! large-company guard) and info rows, plus the decisions that write
//! overrides. Spec: `docs/tech-spec/entity-resolution.md` section 8
//! (P1-CLI-05 core side).
//!
//! Owned by the "resolve" stream. Decisions only write `override` rows; the
//! caller (`Graph`) then runs incremental re-resolution in the same
//! transaction.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use super::clusters::{self, Observations};
use super::rules::{DataFiles, CONF_LARGE_COMPANY, CONF_RARE_NAME};
use super::MatchRule;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::json::{
    round3, FlaggedMerge, Identity, PersonRef, ReviewInfo, ReviewQueue, SourceRefJson, Suggestion,
};
use crate::overrides::{self, OverrideField};
use crate::{IdentityId, PersonId};

/// Filter for bulk confirmation (`review --confirm-all`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfirmFilter {
    /// Only this rule (`name_company` / `name_thread`).
    pub rule: Option<String>,
    /// Only flagged merges at or above this confidence.
    pub min_confidence: Option<f64>,
}

/// A person row as the queue shows it.
fn person_ref(conn: &Connection, id: PersonId) -> Result<PersonRef> {
    Ok(conn
        .prepare_cached("SELECT id, display_name, title, company, score FROM person WHERE id = ?1")?
        .query_row([id], |r| {
            Ok(PersonRef {
                id: r.get(0)?,
                display_name: r.get(1)?,
                title: r.get(2)?,
                company: r.get(3)?,
                score: round3(r.get(4)?),
            })
        })?)
}

/// Every identity of a person, as JSON.
fn identities(conn: &Connection, person: PersonId) -> Result<Vec<Identity>> {
    let mut st = conn.prepare_cached(
        "SELECT i.id, i.kind, i.value, i.confidence, i.match_rule, s.id, s.kind, s.label
         FROM identity i JOIN source s ON s.id = i.source_id
         WHERE i.person_id = ?1 ORDER BY i.id",
    )?;
    let rows = st
        .query_map([person], |r| {
            Ok(Identity {
                id: r.get(0)?,
                kind: r.get(1)?,
                value: r.get(2)?,
                confidence: r.get(3)?,
                match_rule: r.get(4)?,
                source: SourceRefJson {
                    id: r.get(5)?,
                    kind: r.get(6)?,
                    label: r.get(7)?,
                },
                verification: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn is_flagged_rule(rule: Option<&str>) -> bool {
    rule.and_then(MatchRule::parse)
        .is_some_and(MatchRule::is_flagged)
}

/// People holding flagged identities, by score (desc) then id.
fn flagged_people(conn: &Connection) -> Result<Vec<PersonId>> {
    Ok(conn
        .prepare(
            "SELECT p.id FROM person p
             WHERE EXISTS (SELECT 1 FROM identity i WHERE i.person_id = p.id
                           AND i.match_rule IN ('name_company', 'name_thread'))
             ORDER BY p.score DESC, p.id",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Per-person matching attributes for suggestions and explanations.
#[derive(Debug, Default)]
struct PersonAttrs {
    names: BTreeSet<String>,
    companies: BTreeSet<String>,
    threads: BTreeSet<String>,
    identities: Vec<IdentityId>,
}

fn person_attrs(
    conn: &Connection,
    obs: &Observations,
) -> Result<(BTreeMap<PersonId, PersonAttrs>, BTreeMap<PersonId, f64>)> {
    let mut assignment: BTreeMap<IdentityId, PersonId> = BTreeMap::new();
    let mut st = conn.prepare(
        "SELECT i.id, i.person_id FROM identity i JOIN person p ON p.id = i.person_id
         WHERE p.is_self = 0 AND p.is_bulk = 0",
    )?;
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (i, p) = row?;
        assignment.insert(i, p);
    }
    let mut attrs: BTreeMap<PersonId, PersonAttrs> = BTreeMap::new();
    for (&i, &p) in &assignment {
        let a = attrs.entry(p).or_default();
        a.identities.push(i);
        if let Some(n) = obs.names.get(&i) {
            a.names.extend(n.iter().cloned());
        }
        if let Some(c) = obs.companies.get(&i) {
            a.companies.extend(c.iter().cloned());
        }
    }
    for (t, ids) in &obs.threads {
        for i in ids {
            if let Some(p) = assignment.get(i) {
                attrs.entry(*p).or_default().threads.insert(t.clone());
            }
        }
    }
    let scores: BTreeMap<PersonId, f64> = conn
        .prepare("SELECT id, score FROM person")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((attrs, scores))
}

/// `(a, b)` pairs of people separated by a `split_from`, as person ids.
fn separated(conn: &Connection) -> Result<BTreeSet<(PersonId, PersonId)>> {
    let mut st = conn.prepare(
        "SELECT a.person_id, b.person_id FROM override o
         JOIN identity a ON a.id = o.person_id
         JOIN identity b ON b.id = CAST(o.value AS INTEGER)
         WHERE o.field = 'split_from' AND a.person_id IS NOT NULL AND b.person_id IS NOT NULL",
    )?;
    let mut out = BTreeSet::new();
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (a, b) = row?;
        out.insert((a.min(b), a.max(b)));
    }
    Ok(out)
}

fn quoted(set: impl IntoIterator<Item = String>) -> String {
    set.into_iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rule 5, large-company guard and Jaro-Winkler suggestions (section 3).
fn suggestions(
    conn: &Connection,
    cfg: &Config,
    data: &DataFiles,
    obs: &Observations,
) -> Result<Vec<Suggestion>> {
    let (attrs, scores) = person_attrs(conn, obs)?;
    let blocked = separated(conn)?;
    let mut company_size: BTreeMap<&str, u32> = BTreeMap::new();
    for a in attrs.values() {
        for c in &a.companies {
            *company_size.entry(c.as_str()).or_insert(0) += 1;
        }
    }
    let mut by_name: BTreeMap<&str, Vec<PersonId>> = BTreeMap::new();
    for (p, a) in &attrs {
        for n in &a.names {
            by_name.entry(n.as_str()).or_default().push(*p);
        }
    }
    let mut seen: BTreeSet<(PersonId, PersonId)> = blocked;
    // (a, b, rule, confidence, why)
    let mut found: Vec<(PersonId, PersonId, &'static str, f64, String)> = Vec::new();
    for (name, people) in &by_name {
        let common = data.is_common_name(name);
        for (i, &a) in people.iter().enumerate() {
            for &b in &people[i + 1..] {
                let key = (a.min(b), a.max(b));
                if seen.contains(&key) {
                    continue;
                }
                if common {
                    let shared: Vec<String> = attrs[&a]
                        .companies
                        .intersection(&attrs[&b].companies)
                        .filter(|c| {
                            company_size.get(c.as_str()).copied().unwrap_or(0)
                                >= data.large_company_threshold
                        })
                        .cloned()
                        .collect();
                    if shared.is_empty() {
                        continue;
                    }
                    seen.insert(key);
                    found.push((
                        key.0,
                        key.1,
                        "large_company",
                        CONF_LARGE_COMPANY,
                        format!(
                            "same common name \"{name}\" at a large company ({})",
                            quoted(shared)
                        ),
                    ));
                } else {
                    seen.insert(key);
                    found.push((
                        key.0,
                        key.1,
                        "rare_name",
                        CONF_RARE_NAME,
                        format!("identical uncommon name \"{name}\"; no shared company, thread or handle"),
                    ));
                }
            }
        }
    }
    // Jaro-Winkler, blocked by last token and by the first 3 characters.
    let mut blocks: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for name in by_name.keys() {
        if let Some(last) = name.split_whitespace().last() {
            blocks.entry(format!("l:{last}")).or_default().insert(name);
        }
        let head: String = name.chars().take(3).collect();
        blocks.entry(format!("h:{head}")).or_default().insert(name);
    }
    let threshold = cfg.resolution.suggest_jaro_winkler;
    let mut compared: BTreeSet<(&str, &str)> = BTreeSet::new();
    for names in blocks.values() {
        let names: Vec<&str> = names.iter().copied().collect();
        for (i, &n1) in names.iter().enumerate() {
            for &n2 in &names[i + 1..] {
                if !compared.insert((n1, n2)) {
                    continue;
                }
                let jw = strsim::jaro_winkler(n1, n2);
                if jw < threshold {
                    continue;
                }
                let both_rare = !data.is_common_name(n1) && !data.is_common_name(n2);
                for &a in &by_name[n1] {
                    for &b in &by_name[n2] {
                        let key = (a.min(b), a.max(b));
                        if a == b || seen.contains(&key) {
                            continue;
                        }
                        let (x, y) = (&attrs[&a], &attrs[&b]);
                        let company = x.companies.intersection(&y.companies).next().is_some();
                        let thread = x.threads.intersection(&y.threads).next().is_some();
                        if !(company || thread || both_rare) {
                            continue;
                        }
                        seen.insert(key);
                        let mut why =
                            format!("similar names \"{n1}\" / \"{n2}\" (jaro-winkler {jw:.4})");
                        if company {
                            why.push_str("; shared company");
                        }
                        if thread {
                            why.push_str("; shared thread");
                        }
                        found.push((key.0, key.1, "jaro_winkler", CONF_RARE_NAME * jw, why));
                    }
                }
            }
        }
    }
    let score = |p: &PersonId| scores.get(p).copied().unwrap_or(0.0);
    found.sort_by(|x, y| {
        let sx = score(&x.0).max(score(&x.1));
        let sy = score(&y.0).max(score(&y.1));
        sy.total_cmp(&sx).then((x.0, x.1).cmp(&(y.0, y.1)))
    });
    found
        .into_iter()
        .map(|(a, b, rule, confidence, why)| {
            Ok(Suggestion {
                a: person_ref(conn, a)?,
                b: person_ref(conn, b)?,
                rule: rule.to_string(),
                confidence: (confidence * 10_000.0).round() / 10_000.0,
                why,
            })
        })
        .collect()
}

fn flagged_merge(conn: &Connection, obs: &Observations, person: PersonId) -> Result<FlaggedMerge> {
    let all = identities(conn, person)?;
    let (flagged, other): (Vec<Identity>, Vec<Identity>) = all
        .into_iter()
        .partition(|i| is_flagged_rule(i.match_rule.as_deref()));
    let weakest = flagged
        .iter()
        .min_by(|a, b| {
            a.confidence
                .unwrap_or(0.0)
                .total_cmp(&b.confidence.unwrap_or(0.0))
                .then(a.id.cmp(&b.id))
        })
        .map(|i| {
            (
                i.match_rule.clone().unwrap_or_default(),
                i.confidence.unwrap_or(0.0),
            )
        })
        .unwrap_or_default();
    let set = |ids: &[Identity], m: &BTreeMap<IdentityId, BTreeSet<String>>| -> BTreeSet<String> {
        ids.iter()
            .filter_map(|i| m.get(&i.id))
            .flatten()
            .cloned()
            .collect()
    };
    let names: Vec<String> = set(&flagged, &obs.names)
        .intersection(&set(&other, &obs.names))
        .cloned()
        .collect();
    let mut why = format!("same name {}", quoted(names));
    if weakest.0 == "name_company" {
        let companies: Vec<String> = set(&flagged, &obs.companies)
            .intersection(&set(&other, &obs.companies))
            .cloned()
            .collect();
        why.push_str(&format!("; company {}", quoted(companies)));
    } else {
        why.push_str("; both on one email thread or meeting");
    }
    Ok(FlaggedMerge {
        person: person_ref(conn, person)?,
        rule: weakest.0,
        confidence: weakest.1,
        flagged_identities: flagged,
        other_identities: other,
        why,
    })
}

/// Build the queue, sorted by the higher score of the people involved.
pub fn queue(conn: &Connection, cfg: &Config) -> Result<ReviewQueue> {
    let data = DataFiles::load(&cfg.resolution);
    let obs = Observations::load(conn, &data)?;
    let flagged = flagged_people(conn)?
        .into_iter()
        .map(|p| flagged_merge(conn, &obs, p))
        .collect::<Result<Vec<_>>>()?;
    let suggestions = suggestions(conn, cfg, &data, &obs)?;
    let mut info = Vec::new();
    let mut cards: BTreeMap<IdentityId, usize> = BTreeMap::new();
    for g in &obs.coref {
        for id in g.iter().filter(|i| obs.shared.contains(i)) {
            *cards.entry(*id).or_insert(0) += 1;
        }
    }
    for (id, n) in cards {
        if n > 1 {
            let value = obs
                .identities
                .get(&id)
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            info.push(ReviewInfo {
                kind: "shared_handle".into(),
                message: format!("{value} is on {n} cards; kept separate"),
            });
        }
    }
    for id in clusters::resolve(conn, cfg)?.conflicting_overrides {
        info.push(ReviewInfo {
            kind: "conflicting_override".into(),
            message: format!("conflicting override #{id} skipped (a split_from forbids it)"),
        });
    }
    Ok(ReviewQueue {
        v: crate::json::V,
        flagged,
        suggestions,
        info,
    })
}

/// Confirm a flagged merge: `merged_into` pins each flagged identity to the
/// anchor. Returns the override ids written.
pub fn confirm(conn: &Connection, person: PersonId, now: i64) -> Result<Vec<i64>> {
    let flagged: Vec<IdentityId> = conn
        .prepare_cached(
            "SELECT id FROM identity WHERE person_id = ?1
             AND match_rule IN ('name_company', 'name_thread') ORDER BY id",
        )?
        .query_map([person], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if flagged.is_empty() {
        return Err(Error::BadRequest(format!(
            "person {person} has no flagged merge to confirm"
        )));
    }
    let mut out = Vec::new();
    for id in flagged {
        if let Some(o) = overrides::add_multi(
            conn,
            id,
            OverrideField::MergedInto,
            &person.to_string(),
            now,
        )? {
            out.push(o);
        }
    }
    Ok(out)
}

/// Confirm every flagged merge matching `filter`. Returns how many people.
pub fn confirm_all(
    conn: &Connection,
    cfg: &Config,
    filter: &ConfirmFilter,
    now: i64,
) -> Result<usize> {
    let _ = cfg;
    let mut n = 0;
    for p in flagged_people(conn)? {
        let rows: Vec<(String, f64)> = conn
            .prepare_cached(
                "SELECT match_rule, confidence FROM identity WHERE person_id = ?1
                 AND match_rule IN ('name_company', 'name_thread')",
            )?
            .query_map([p], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let rule_ok = filter
            .rule
            .as_deref()
            .is_none_or(|want| rows.iter().all(|(r, _)| r == want));
        let min_conf = rows.iter().map(|(_, c)| *c).fold(f64::INFINITY, f64::min);
        let conf_ok = filter.min_confidence.is_none_or(|m| min_conf >= m - 1e-9);
        if rule_ok && conf_ok {
            confirm(conn, p, now)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Reject a suggestion: `split_from` between the two anchors. Returns the
/// override id (the existing one when already rejected).
pub fn reject(conn: &Connection, a: PersonId, b: PersonId, now: i64) -> Result<i64> {
    if a == b {
        return Err(Error::BadRequest(
            "cannot separate a person from itself".into(),
        ));
    }
    match overrides::add_multi(conn, a, OverrideField::SplitFrom, &b.to_string(), now)? {
        Some(id) => Ok(id),
        None => Ok(conn.query_row(
            "SELECT id FROM override WHERE person_id = ?1 AND field = 'split_from' AND value = ?2",
            rusqlite::params![a, b.to_string()],
            |r| r.get(0),
        )?),
    }
}
