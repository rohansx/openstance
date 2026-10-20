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

use crate::error::Result;
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

/// Write a scalar override (`None` = tombstone). Returns `override.id`.
pub fn set_scalar(
    conn: &Connection,
    person: PersonId,
    field: OverrideField,
    value: Option<&str>,
    now: i64,
) -> Result<i64> {
    let _ = (conn, person, field, value, now);
    todo!("resolve stream: P1-CORE-08 set_scalar")
}

/// Add a multi-valued row; `None` when the same value already exists.
pub fn add_multi(
    conn: &Connection,
    person: PersonId,
    field: OverrideField,
    value: &str,
    now: i64,
) -> Result<Option<i64>> {
    let _ = (conn, person, field, value, now);
    todo!("resolve stream: P1-CORE-08 add_multi")
}

/// Delete multi-valued rows with this value on any identity of `cluster`.
pub fn remove_multi(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
    value: &str,
) -> Result<usize> {
    let _ = (conn, cluster, field, value);
    todo!("resolve stream: P1-CORE-08 remove_multi")
}

/// Effective scalar over a cluster: latest `(set_at, id)` row; `None` when
/// there is no row or the latest is a tombstone.
pub fn scalar(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<Option<String>> {
    let _ = (conn, cluster, field);
    todo!("resolve stream: P1-CORE-08 scalar")
}

/// Privacy flag over a cluster: latest value per anchor identity, OR'd.
pub fn privacy_flag(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<bool> {
    let _ = (conn, cluster, field);
    todo!("resolve stream: P1-CORE-08 privacy_flag")
}

/// Every row of a multi-valued field over a cluster, newest first.
pub fn multi(
    conn: &Connection,
    cluster: &[IdentityId],
    field: OverrideField,
) -> Result<Vec<OverrideRow>> {
    let _ = (conn, cluster, field);
    todo!("resolve stream: P1-CORE-08 multi")
}

/// Every override row over a cluster (`edit --history`), oldest first.
pub fn history(conn: &Connection, cluster: &[IdentityId]) -> Result<Vec<OverrideRow>> {
    let _ = (conn, cluster);
    todo!("resolve stream: P1-CORE-08 history")
}

/// Apply an [`Edit`] to `person` (an anchor id). Returns the override ids
/// written. Does not recompute derived data.
pub fn apply_edit(conn: &Connection, person: PersonId, edit: &Edit, now: i64) -> Result<Vec<i64>> {
    let _ = (conn, person, edit, now);
    todo!("resolve stream: P1-CORE-08 apply_edit")
}

#[cfg(test)]
mod tests {
    use super::*;

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
