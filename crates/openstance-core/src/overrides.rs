//! The `override` table: user edits that survive every rebuild.
//! Scalars (`display_name, title, company, location, bio, do_not_share,
//! no_enrich, is_bulk`) are last-write-wins by `(set_at, id)` with `NULL` as a
//! tombstone; multi-valued fields (`tag, note, merged_into, split_from`) keep
//! every row and removal deletes the row. Lookups take every identity id in
//! the person's cluster, so edits follow people through merges.
//! Spec: `docs/tech-spec/data-model.md` section 4.7 (P1-CORE-08).
//!
//! Owned by the "resolve" stream.

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::{IdentityId, PersonId};

/// An `override.field` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideField {
    /// Scalar.
    DisplayName,
    /// Scalar.
    Title,
    /// Scalar.
    Company,
    /// Scalar.
    Location,
    /// Scalar.
    Bio,
    /// Scalar flag `'1'`/`'0'` (OR'd across the cluster).
    DoNotShare,
    /// Scalar flag `'1'`/`'0'` (OR'd across the cluster).
    NoEnrich,
    /// Scalar flag `'1'`/`'0'` (`edit --not-bulk`).
    IsBulk,
    /// Multi-valued, lowercased.
    Tag,
    /// Multi-valued free text.
    Note,
    /// Multi-valued must-link (value = other anchor id).
    MergedInto,
    /// Multi-valued cannot-link (value = other anchor id).
    SplitFrom,
}

impl OverrideField {
    /// Every field, in schema order.
    pub const ALL: [OverrideField; 12] = [
        OverrideField::DisplayName,
        OverrideField::Title,
        OverrideField::Company,
        OverrideField::Location,
        OverrideField::Bio,
        OverrideField::DoNotShare,
        OverrideField::NoEnrich,
        OverrideField::IsBulk,
        OverrideField::Tag,
        OverrideField::Note,
        OverrideField::MergedInto,
        OverrideField::SplitFrom,
    ];

    /// The string stored in `override.field`.
    pub fn as_str(self) -> &'static str {
        match self {
            OverrideField::DisplayName => "display_name",
            OverrideField::Title => "title",
            OverrideField::Company => "company",
            OverrideField::Location => "location",
            OverrideField::Bio => "bio",
            OverrideField::DoNotShare => "do_not_share",
            OverrideField::NoEnrich => "no_enrich",
            OverrideField::IsBulk => "is_bulk",
            OverrideField::Tag => "tag",
            OverrideField::Note => "note",
            OverrideField::MergedInto => "merged_into",
            OverrideField::SplitFrom => "split_from",
        }
    }

    /// Parse a stored field name.
    pub fn parse(s: &str) -> Option<OverrideField> {
        OverrideField::ALL.into_iter().find(|f| f.as_str() == s)
    }

    /// Last-write-wins scalar (as opposed to multi-valued).
    pub fn is_scalar(self) -> bool {
        !matches!(
            self,
            OverrideField::Tag
                | OverrideField::Note
                | OverrideField::MergedInto
                | OverrideField::SplitFrom
        )
    }
}

/// One `override` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideRow {
    /// `override.id`.
    pub id: i64,
    /// Anchor identity id the edit was written against.
    pub person_id: IdentityId,
    /// Field.
    pub field: OverrideField,
    /// Value; `None` is a scalar tombstone.
    pub value: Option<String>,
    /// Unix seconds.
    pub set_at: i64,
}

/// A batch of field edits for one person (`openstance edit`, `POST /edits`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edit {
    /// Scalar sets (`--title CTO`). Empty string = explicitly empty.
    pub set: Vec<(OverrideField, String)>,
    /// Scalar tombstones (`--unset location`).
    pub unset: Vec<OverrideField>,
    /// `--tag`.
    pub add_tags: Vec<String>,
    /// `--untag`.
    pub remove_tags: Vec<String>,
    /// `--note`.
    pub add_notes: Vec<String>,
    /// `--no-share` (true) / `--share` (false).
    pub do_not_share: Option<bool>,
    /// `--no-enrich` (true) / `--enrich` (false).
    pub no_enrich: Option<bool>,
    /// `--bulk` (true) / `--not-bulk` (false).
    pub is_bulk: Option<bool>,
}

/// Every identity id in `person`'s cluster (`[person]` when the id is not
/// a current person, so edits on an edit-only anchor still resolve).
pub fn cluster(conn: &Connection, person: PersonId) -> Result<Vec<IdentityId>> {
    let mut st = conn.prepare_cached("SELECT id FROM identity WHERE person_id = ?1 ORDER BY id")?;
    let ids = st
        .query_map([person], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(if ids.is_empty() { vec![person] } else { ids })
}

fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

fn read_rows(
    conn: &Connection,
    cluster: &[IdentityId],
    field: Option<OverrideField>,
    order: &str,
) -> Result<Vec<OverrideRow>> {
    if cluster.is_empty() {
        return Ok(Vec::new());
    }
    let field_clause = if field.is_some() {
        " AND field = ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, person_id, field, value, set_at FROM override
         WHERE person_id IN ({}){field_clause} ORDER BY {order}",
        placeholders(cluster.len())
    );
    let mut params: Vec<rusqlite::types::Value> = cluster
        .iter()
        .map(|&i| rusqlite::types::Value::Integer(i))
        .collect();
    if let Some(f) = field {
        params.push(rusqlite::types::Value::Text(f.as_str().to_string()));
    }
    let mut st = conn.prepare(&sql)?;
    let rows = st
        .query_map(rusqlite::params_from_iter(params), |r| {
            let f: String = r.get(2)?;
            Ok((r.get(0)?, r.get(1)?, f, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<Vec<(i64, i64, String, Option<String>, i64)>>>()?;
    rows.into_iter()
        .map(|(id, person_id, f, value, set_at)| {
            let field = OverrideField::parse(&f)
                .ok_or_else(|| Error::Internal(format!("unknown override field in row {id}")))?;
            Ok(OverrideRow {
                id,
                person_id,
                field,
                value,
                set_at,
            })
        })
        .collect()
}

fn insert(
    conn: &Connection,
    person: PersonId,
    field: OverrideField,
    value: Option<&str>,
    now: i64,
) -> Result<i64> {
    conn.prepare_cached(
        "INSERT INTO override(person_id, field, value, set_at) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(rusqlite::params![person, field.as_str(), value, now])?;
    Ok(conn.last_insert_rowid())
}

/// Write a scalar override (`None` = tombstone). Returns `override.id`.
pub fn set_scalar(
    conn: &Connection,
    person: PersonId,
    field: OverrideField,
    value: Option<&str>,
    now: i64,
) -> Result<i64> {
    if !field.is_scalar() {
        return Err(Error::BadRequest(format!(
            "{} is multi-valued, not a scalar",
            field.as_str()
        )));
    }
    insert(conn, person, field, value, now)
}

/// Add a multi-valued row; `None` when the same value already exists.
/// Tags are trimmed and lowercased. Notes may repeat.
pub fn add_multi(
    conn: &Connection,
    person: PersonId,
    field: OverrideField,
    value: &str,
    now: i64,
) -> Result<Option<i64>> {
    if field.is_scalar() {
        return Err(Error::BadRequest(format!(
            "{} is a scalar, not multi-valued",
            field.as_str()
        )));
    }
    let value = match field {
        OverrideField::Tag => value.trim().to_lowercase(),
        _ => value.trim().to_string(),
    };
    if value.is_empty() {
        return Err(Error::BadRequest(format!("empty {}", field.as_str())));
    }
    if field != OverrideField::Note {
        let exists: bool = conn
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM override WHERE person_id = ?1 AND field = ?2 AND value = ?3)",
            )?
            .query_row(rusqlite::params![person, field.as_str(), value], |r| r.get(0))?;
        if exists {
            return Ok(None);
        }
    }
    insert(conn, person, field, Some(&value), now).map(Some)
}

/// Delete multi-valued rows with this value on any identity of `cluster`.
pub fn remove_multi(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
    value: &str,
) -> Result<usize> {
    if field.is_scalar() || cluster.is_empty() {
        return Ok(0);
    }
    let value = match field {
        OverrideField::Tag => value.trim().to_lowercase(),
        _ => value.trim().to_string(),
    };
    let sql = format!(
        "DELETE FROM override WHERE field = ? AND value = ? AND person_id IN ({})",
        placeholders(cluster.len())
    );
    let mut params: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::Text(field.as_str().to_string()),
        rusqlite::types::Value::Text(value),
    ];
    params.extend(cluster.iter().map(|&i| rusqlite::types::Value::Integer(i)));
    Ok(conn.execute(&sql, rusqlite::params_from_iter(params))?)
}

/// Effective scalar over a cluster: latest `(set_at, id)` row; `None` when
/// there is no row or the latest is a tombstone.
pub fn scalar(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<Option<String>> {
    let rows = read_rows(conn, cluster, Some(field), "set_at DESC, id DESC")?;
    Ok(rows.into_iter().next().and_then(|r| r.value))
}

/// Privacy flag over a cluster: latest value per anchor identity, OR'd.
pub fn privacy_flag(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<bool> {
    Ok(!flagged_anchors(conn, cluster, field)?.is_empty())
}

/// Identities of `cluster` whose latest value of `field` is `'1'`.
fn flagged_anchors(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<Vec<IdentityId>> {
    let rows = read_rows(
        conn,
        cluster,
        Some(field),
        "person_id, set_at DESC, id DESC",
    )?;
    let mut out = Vec::new();
    let mut last: Option<i64> = None;
    for r in rows {
        if last == Some(r.person_id) {
            continue;
        }
        last = Some(r.person_id);
        if r.value.as_deref() == Some("1") {
            out.push(r.person_id);
        }
    }
    Ok(out)
}

/// Every row of a multi-valued field over a cluster, newest first.
pub fn multi(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<Vec<OverrideRow>> {
    read_rows(conn, cluster, Some(field), "set_at DESC, id DESC")
}

/// Every override row over a cluster (`edit --history`), oldest first.
pub fn history(conn: &Connection, cluster: &[IdentityId]) -> Result<Vec<OverrideRow>> {
    read_rows(conn, cluster, None, "set_at, id")
}

fn set_privacy(
    conn: &Connection,
    person: PersonId,
    cluster: &[IdentityId],
    field: OverrideField,
    on: bool,
    now: i64,
    out: &mut Vec<i64>,
) -> Result<()> {
    if on {
        out.push(set_scalar(conn, person, field, Some("1"), now)?);
    } else {
        // Clearing writes '0' for every identity carrying a '1', so the
        // OR across the cluster becomes false.
        for anchor in flagged_anchors(conn, cluster, field)? {
            out.push(set_scalar(conn, anchor, field, Some("0"), now)?);
        }
    }
    Ok(())
}

/// Apply an [`Edit`] to `person` (an anchor id). Returns the override ids
/// written. Does not recompute derived data.
pub fn apply_edit(conn: &Connection, person: PersonId, edit: &Edit, now: i64) -> Result<Vec<i64>> {
    let cluster = cluster(conn, person)?;
    let mut out = Vec::new();
    for (field, value) in &edit.set {
        out.push(set_scalar(conn, person, *field, Some(value), now)?);
    }
    for field in &edit.unset {
        out.push(set_scalar(conn, person, *field, None, now)?);
    }
    for tag in &edit.add_tags {
        if let Some(id) = add_multi(conn, person, OverrideField::Tag, tag, now)? {
            out.push(id);
        }
    }
    for tag in &edit.remove_tags {
        remove_multi(conn, &cluster, OverrideField::Tag, tag)?;
    }
    for note in &edit.add_notes {
        if let Some(id) = add_multi(conn, person, OverrideField::Note, note, now)? {
            out.push(id);
        }
    }
    if let Some(on) = edit.do_not_share {
        set_privacy(
            conn,
            person,
            &cluster,
            OverrideField::DoNotShare,
            on,
            now,
            &mut out,
        )?;
    }
    if let Some(on) = edit.no_enrich {
        set_privacy(
            conn,
            person,
            &cluster,
            OverrideField::NoEnrich,
            on,
            now,
            &mut out,
        )?;
    }
    if let Some(on) = edit.is_bulk {
        let v = if on { "1" } else { "0" };
        out.push(set_scalar(
            conn,
            person,
            OverrideField::IsBulk,
            Some(v),
            now,
        )?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_in_memory};

    fn conn() -> Connection {
        let mut c = open_in_memory().unwrap();
        migrate(&mut c, None, 0).unwrap();
        c
    }

    #[test]
    fn later_title_edit_wins() {
        let c = conn();
        set_scalar(&c, 1, OverrideField::Title, Some("CTO"), 10).unwrap();
        set_scalar(&c, 1, OverrideField::Title, Some("CEO"), 20).unwrap();
        assert_eq!(
            scalar(&c, &[1], OverrideField::Title).unwrap().as_deref(),
            Some("CEO")
        );
        // Same set_at: the higher id wins.
        set_scalar(&c, 1, OverrideField::Title, Some("Founder"), 20).unwrap();
        assert_eq!(
            scalar(&c, &[1], OverrideField::Title).unwrap().as_deref(),
            Some("Founder")
        );
    }

    #[test]
    fn tombstone_restores_the_derived_value() {
        let c = conn();
        set_scalar(&c, 1, OverrideField::Title, Some("CTO"), 10).unwrap();
        set_scalar(&c, 1, OverrideField::Title, None, 11).unwrap();
        assert_eq!(scalar(&c, &[1], OverrideField::Title).unwrap(), None);
        // History keeps both rows.
        let h = history(&c, &[1]).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].value.as_deref(), Some("CTO"));
        assert_eq!(h[1].value, None);
    }

    #[test]
    fn empty_string_is_an_explicit_value_not_a_tombstone() {
        let c = conn();
        set_scalar(&c, 1, OverrideField::Company, Some(""), 10).unwrap();
        assert_eq!(
            scalar(&c, &[1], OverrideField::Company).unwrap().as_deref(),
            Some("")
        );
    }

    #[test]
    fn scalars_follow_the_cluster() {
        let c = conn();
        set_scalar(&c, 7, OverrideField::Title, Some("from B"), 10).unwrap();
        set_scalar(&c, 3, OverrideField::Location, Some("Pune"), 11).unwrap();
        assert_eq!(
            scalar(&c, &[3, 7], OverrideField::Title)
                .unwrap()
                .as_deref(),
            Some("from B")
        );
        assert_eq!(scalar(&c, &[3], OverrideField::Title).unwrap(), None);
    }

    #[test]
    fn two_tags_survive_and_one_can_be_removed() {
        let c = conn();
        let a = add_multi(&c, 1, OverrideField::Tag, "investor", 10).unwrap();
        let b = add_multi(&c, 1, OverrideField::Tag, "angel", 11).unwrap();
        assert!(a.is_some() && b.is_some());
        assert_eq!(
            add_multi(&c, 1, OverrideField::Tag, "investor", 12).unwrap(),
            None
        );
        let tags: Vec<_> = multi(&c, &[1], OverrideField::Tag)
            .unwrap()
            .into_iter()
            .map(|r| r.value.unwrap())
            .collect();
        assert_eq!(tags, ["angel", "investor"], "newest first");
        assert_eq!(
            remove_multi(&c, &[1], OverrideField::Tag, "investor").unwrap(),
            1
        );
        let tags: Vec<_> = multi(&c, &[1], OverrideField::Tag)
            .unwrap()
            .into_iter()
            .map(|r| r.value.unwrap())
            .collect();
        assert_eq!(tags, ["angel"]);
    }

    #[test]
    fn notes_may_repeat() {
        let c = conn();
        assert!(add_multi(&c, 1, OverrideField::Note, "met at dinner", 1)
            .unwrap()
            .is_some());
        assert!(add_multi(&c, 1, OverrideField::Note, "met at dinner", 2)
            .unwrap()
            .is_some());
        assert_eq!(multi(&c, &[1], OverrideField::Note).unwrap().len(), 2);
    }

    #[test]
    fn scalar_and_multi_apis_reject_the_wrong_kind() {
        let c = conn();
        assert!(set_scalar(&c, 1, OverrideField::Tag, Some("x"), 1).is_err());
        assert!(add_multi(&c, 1, OverrideField::Title, "x", 1).is_err());
    }

    #[test]
    fn privacy_flags_are_or_ed_per_anchor() {
        // ER-18: '1' on B, a newer explicit '0' on A: still 1 after a merge.
        let c = conn();
        set_scalar(&c, 9, OverrideField::DoNotShare, Some("1"), 10).unwrap();
        set_scalar(&c, 2, OverrideField::DoNotShare, Some("0"), 20).unwrap();
        assert!(privacy_flag(&c, &[2, 9], OverrideField::DoNotShare).unwrap());
        assert!(!privacy_flag(&c, &[2], OverrideField::DoNotShare).unwrap());
        // A later '0' on B itself clears B's contribution.
        set_scalar(&c, 9, OverrideField::DoNotShare, Some("0"), 30).unwrap();
        assert!(!privacy_flag(&c, &[2, 9], OverrideField::DoNotShare).unwrap());
    }

    #[test]
    fn apply_edit_writes_one_row_per_change() {
        let c = conn();
        c.execute_batch(
            "INSERT INTO source(id, kind, imported_at) VALUES (1, 'vcf', 0);
             INSERT INTO person(id) VALUES (1);
             INSERT INTO identity(id, kind, value, source_id, person_id) VALUES
               (1, 'email', 'a@example.com', 1, 1), (2, 'email', 'b@example.com', 1, 1);",
        )
        .unwrap();
        set_scalar(&c, 2, OverrideField::NoEnrich, Some("1"), 1).unwrap();
        let ids = apply_edit(
            &c,
            1,
            &Edit {
                set: vec![(OverrideField::Title, "CTO".into())],
                unset: vec![OverrideField::Location],
                add_tags: vec!["  Investor-Intro ".into()],
                add_notes: vec!["intro via Sam".into()],
                do_not_share: Some(true),
                no_enrich: Some(false),
                ..Edit::default()
            },
            5,
        )
        .unwrap();
        // title, location tombstone, tag, note, do_not_share, no_enrich '0' on #2
        assert_eq!(ids.len(), 6);
        assert_eq!(
            scalar(&c, &[1, 2], OverrideField::Title)
                .unwrap()
                .as_deref(),
            Some("CTO")
        );
        assert_eq!(
            multi(&c, &[1], OverrideField::Tag).unwrap()[0]
                .value
                .as_deref(),
            Some("investor-intro")
        );
        assert!(privacy_flag(&c, &[1, 2], OverrideField::DoNotShare).unwrap());
        assert!(!privacy_flag(&c, &[1, 2], OverrideField::NoEnrich).unwrap());
        let ids = apply_edit(
            &c,
            1,
            &Edit {
                remove_tags: vec!["investor-intro".into()],
                ..Edit::default()
            },
            6,
        )
        .unwrap();
        assert!(ids.is_empty());
        assert!(multi(&c, &[1, 2], OverrideField::Tag).unwrap().is_empty());
    }

    #[test]
    fn field_names_match_the_schema_check_constraint() {
        let names: Vec<_> = OverrideField::ALL.iter().map(|f| f.as_str()).collect();
        assert_eq!(
            names,
            [
                "display_name",
                "title",
                "company",
                "location",
                "bio",
                "do_not_share",
                "no_enrich",
                "is_bulk",
                "tag",
                "note",
                "merged_into",
                "split_from"
            ]
        );
        assert_eq!(OverrideField::parse("tag"), Some(OverrideField::Tag));
        assert!(OverrideField::Title.is_scalar());
        assert!(!OverrideField::SplitFrom.is_scalar());
    }
}
