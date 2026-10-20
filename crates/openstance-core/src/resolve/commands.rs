//! `merge` and `split`: override writes with the anchor-swap and pinning
//! rules. Spec: `docs/tech-spec/entity-resolution.md` section 9 (P1-CLI-06
//! core side).
//!
//! Owned by the "resolve" stream. These functions only write overrides; the
//! caller runs incremental re-resolution afterwards.

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::overrides::{self, OverrideField};
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

fn id_list(ids: &[IdentityId]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

/// Delete `field` rows linking any identity of `a` with any identity of `b`
/// (either direction).
fn delete_links(
    conn: &Connection,
    field: OverrideField,
    a: &[IdentityId],
    b: &[IdentityId],
) -> Result<usize> {
    if a.is_empty() || b.is_empty() {
        return Ok(0);
    }
    let (la, lb) = (id_list(a), id_list(b));
    let sql = format!(
        "DELETE FROM override WHERE field = ?1 AND (
           (person_id IN ({la}) AND CAST(value AS INTEGER) IN ({lb}))
        OR (person_id IN ({lb}) AND CAST(value AS INTEGER) IN ({la})))"
    );
    Ok(conn.execute(&sql, [field.as_str()])?)
}

/// Merge `people` (at least two distinct persons) into the first.
pub fn merge(conn: &Connection, people: &[PersonId], now: i64) -> Result<MergeOutcome> {
    if people.len() < 2 {
        return Err(Error::BadRequest("merge needs at least two people".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !people.iter().all(|p| seen.insert(*p)) {
        return Err(Error::BadRequest(
            "two of the references name the same person".into(),
        ));
    }
    let clusters: Vec<Vec<IdentityId>> = people
        .iter()
        .map(|&p| overrides::cluster(conn, p))
        .collect::<Result<_>>()?;
    let mut split_rows_deleted = 0;
    for i in 0..clusters.len() {
        for j in i + 1..clusters.len() {
            split_rows_deleted +=
                delete_links(conn, OverrideField::SplitFrom, &clusters[i], &clusters[j])?;
        }
    }
    let first = people[0];
    let mut written = Vec::new();
    for &b in &people[1..] {
        if let Some(id) =
            overrides::add_multi(conn, b, OverrideField::MergedInto, &first.to_string(), now)?
        {
            written.push(id);
        }
    }
    let survivor = *people.iter().min().unwrap_or(&first);
    let all: Vec<IdentityId> = clusters.iter().flatten().copied().collect();
    for field in [OverrideField::DoNotShare, OverrideField::NoEnrich] {
        if overrides::privacy_flag(conn, &all, field)? {
            written.push(overrides::set_scalar(
                conn,
                survivor,
                field,
                Some("1"),
                now,
            )?);
        }
    }
    Ok(MergeOutcome {
        person: survivor,
        overrides: written,
        split_rows_deleted,
    })
}

/// `merge --undo`: delete the `merged_into` rows written between the two
/// anchor ids `a` and `b` (as given, not resolved: after a merge both
/// resolve to the same person). Rule-based merges are undone with `split`.
pub fn unmerge(conn: &Connection, a: PersonId, b: PersonId) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM override WHERE field = 'merged_into' AND (
           (person_id = ?1 AND CAST(value AS INTEGER) = ?2)
        OR (person_id = ?2 AND CAST(value AS INTEGER) = ?1))",
        [a, b],
    )?)
}

/// Move `identities` out of `person` into a new person, pinning both sides.
pub fn split(
    conn: &Connection,
    person: PersonId,
    identities: &[IdentityId],
    now: i64,
) -> Result<SplitOutcome> {
    let cluster: Vec<IdentityId> = conn
        .prepare_cached("SELECT id FROM identity WHERE person_id = ?1 ORDER BY id")?
        .query_map([person], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if cluster.is_empty() {
        return Err(Error::NotFound(format!("person {person}")));
    }
    let mut moved: Vec<IdentityId> = identities.to_vec();
    moved.sort_unstable();
    moved.dedup();
    if let Some(bad) = moved.iter().find(|m| !cluster.contains(m)) {
        return Err(Error::BadRequest(format!(
            "identity {bad} is not part of person {person}"
        )));
    }
    if moved.is_empty() || moved.len() == cluster.len() {
        return Err(Error::BadRequest(
            "split needs a non-empty proper subset of the person's identities".into(),
        ));
    }
    let mut kept: Vec<IdentityId> = cluster
        .iter()
        .copied()
        .filter(|c| !moved.contains(c))
        .collect();
    // Keep the anchor on the kept side so existing references stay put.
    let swapped = moved.contains(&person);
    if swapped {
        std::mem::swap(&mut moved, &mut kept);
    }
    let (a_m, a_k) = (moved[0], kept[0]);
    delete_links(conn, OverrideField::MergedInto, &moved, &kept)?;
    let mut written = Vec::new();
    let mut add = |p: IdentityId, field: OverrideField, v: IdentityId| -> Result<()> {
        if let Some(id) = overrides::add_multi(conn, p, field, &v.to_string(), now)? {
            written.push(id);
        }
        Ok(())
    };
    for &m in moved.iter().filter(|&&m| m != a_m) {
        add(m, OverrideField::MergedInto, a_m)?;
    }
    for &k in kept.iter().filter(|&&k| k != a_k) {
        add(k, OverrideField::MergedInto, a_k)?;
    }
    for &m in &moved {
        add(m, OverrideField::SplitFrom, a_k)?;
    }
    Ok(SplitOutcome {
        kept: a_k,
        moved: a_m,
        swapped,
        overrides: written,
    })
}

/// `split --undo`: delete `split_from` rows between them (and the pins with
/// `unpin`).
pub fn unsplit(conn: &Connection, a: PersonId, b: PersonId, unpin: bool) -> Result<usize> {
    let (ca, cb) = (overrides::cluster(conn, a)?, overrides::cluster(conn, b)?);
    let mut n = delete_links(conn, OverrideField::SplitFrom, &ca, &cb)?;
    if unpin {
        for (cluster, anchor) in [(&ca, a), (&cb, b)] {
            n += conn.execute(
                &format!(
                    "DELETE FROM override WHERE field = 'merged_into'
                     AND person_id IN ({}) AND CAST(value AS INTEGER) = ?1",
                    id_list(cluster)
                ),
                [anchor],
            )?;
        }
    }
    Ok(n)
}
