//! Structured filters to SQL over `person` (step 2). Bound parameters only;
//! user strings are never interpolated. Spec: `docs/tech-spec/search.md`
//! section 5.
//!
//! Owned by the "search" stream.

use openstance_model::Query;
use rusqlite::Connection;

use crate::error::Result;
use crate::PersonId;

/// A WHERE clause and its parameters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlFilter {
    /// SQL fragment appended to `SELECT p.id FROM person p WHERE ...`.
    pub sql: String,
    /// Positional parameters.
    pub params: Vec<rusqlite::types::Value>,
}

/// Build the filter for `q` (excludes self, and bulk unless `include_bulk`).
pub fn build(q: &Query) -> SqlFilter {
    let _ = q;
    todo!("search stream: P1-CORE-16 filters::build")
}

/// The allowed ids in structured order (`score DESC, last_contact DESC, id ASC`).
pub fn allowed_ids(conn: &Connection, q: &Query) -> Result<Vec<PersonId>> {
    let _ = (conn, q);
    todo!("search stream: P1-CORE-16 allowed_ids")
}
