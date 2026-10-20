//! Evidence for results: the N most recent contact interactions per person
//! (one windowed query per page) and the rendered evidence line
//! `{YYYY-MM-DD}  {kind} {direction}  "{subject}"  ({source.label})`.
//! Spec: `docs/tech-spec/search.md` section 8.
//!
//! Owned by the "search" stream. Subjects only, never bodies.

use std::collections::BTreeMap;

use openstance_model::Query;
use rusqlite::Connection;

use crate::error::Result;
use crate::json::{Evidence, Matched, Person};
use crate::PersonId;

/// Up to `n` most recent `email/meeting/dm/connection` interactions with
/// `at <= now` per person.
pub fn recent(
    conn: &Connection,
    people: &[PersonId],
    n: usize,
    now: i64,
) -> Result<BTreeMap<PersonId, Vec<Evidence>>> {
    let _ = (conn, people, n, now);
    todo!("search stream: evidence::recent")
}

/// Render one evidence line.
pub fn line(
    at: i64,
    kind: &str,
    direction: Option<&str>,
    subject: Option<&str>,
    source_label: Option<&str>,
) -> String {
    let _ = (at, kind, direction, subject, source_label);
    todo!("search stream: evidence::line")
}

/// Matched-field entries for a person against the parsed query.
pub fn matched_fields(q: &Query, person: &Person) -> Vec<Matched> {
    let _ = (q, person);
    todo!("search stream: matched_fields")
}
