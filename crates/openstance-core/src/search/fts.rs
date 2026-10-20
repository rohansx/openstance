//! `person_fts` maintenance and bm25 candidates. `rowid = person.id`; any
//! change to a person runs delete + insert in the same transaction; bulk and
//! self people are not indexed. Spec: `docs/tech-spec/data-model.md` 4.10
//! and `search.md` 6.1 (P1-CORE-14).
//!
//! Owned by the "search" stream. Columns: `display_name, title, company,
//! location, bio, tags, handles, subjects` (subjects: distinct, newest first,
//! capped at 200 or 8 KB; notes are never indexed).

use rusqlite::Connection;

use crate::error::Result;
use crate::PersonId;

/// Quote every term (doubling embedded quotes) so FTS5 syntax in user input
/// is inert. Terms are OR-ed; 4+ character terms also get a prefix match.
pub fn escape_query(text: &str) -> String {
    let _ = text;
    todo!("search stream: escape_query")
}

/// Delete + insert one person's row (removes it for bulk / self / missing).
pub fn reindex_person(conn: &Connection, id: PersonId) -> Result<()> {
    let _ = (conn, id);
    todo!("search stream: P1-CORE-14 reindex_person")
}

/// [`reindex_person`] for several people.
pub fn reindex_people(conn: &Connection, ids: &[PersonId]) -> Result<()> {
    let _ = (conn, ids);
    todo!("search stream: P1-CORE-14 reindex_people")
}

/// Delete rows for people that no longer exist (absorbed by a merge).
pub fn remove_people(conn: &Connection, ids: &[PersonId]) -> Result<()> {
    let _ = (conn, ids);
    todo!("search stream: P1-CORE-14 remove_people")
}

/// `DELETE FROM person_fts` and reinsert every non-bulk, non-self person.
/// Returns rows written.
pub fn reindex_all(conn: &Connection) -> Result<u64> {
    let _ = conn;
    todo!("search stream: P1-CORE-14 reindex_all")
}

/// Top `limit` bm25 hits for `text` (negated bm25, higher is better),
/// restricted to `allowed` when given.
pub fn candidates(
    conn: &Connection,
    text: &str,
    allowed: Option<&[PersonId]>,
    limit: usize,
) -> Result<Vec<(PersonId, f64)>> {
    let _ = (conn, text, allowed, limit);
    todo!("search stream: P1-CORE-16 fts candidates")
}
