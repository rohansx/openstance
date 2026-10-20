//! Step 15: person fields from importer hints (by source priority), enrich
//! facts (Phase 3) and scalar overrides (LWW; NULL tombstone falls back;
//! privacy flags OR'd per anchor). Spec: `docs/tech-spec/data-model.md`
//! section 4.5 and `entity-resolution.md` section 4.
//!
//! Owned by the "resolve" stream.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::error::Result;
use crate::PersonId;

/// Recompute `display_name, title, company, location, bio, do_not_share,
/// no_enrich` (and `is_bulk` overrides are left to scoring) for `people`, or
/// for everyone when `None`.
pub fn materialize_fields(conn: &Connection, people: Option<&[PersonId]>) -> Result<()> {
    let _ = (conn, people);
    todo!("resolve stream: P1-CORE-08/09 materialize_fields (step 15)")
}

/// Where each displayed scalar of a person came from: `"override"`,
/// `"<source.kind>:<source.label>"`, or `None` (`Person.field_sources`).
pub fn field_sources(
    conn: &Connection,
    person: PersonId,
) -> Result<BTreeMap<String, Option<String>>> {
    let _ = (conn, person);
    todo!("resolve stream: field_sources")
}
