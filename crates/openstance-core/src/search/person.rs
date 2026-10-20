//! Builders for the `Person` and `PersonDetail` JSON shapes
//! (`docs/tech-spec/interfaces.md` 1.2 and 1.8), used by search results,
//! `who` (Phase 2) and export.
//!
//! Owned by the "search" stream.

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::json::{Person, PersonDetail};
use crate::PersonId;

/// Options for [`detail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailOptions {
    /// Recent interactions to include.
    pub interactions: usize,
    /// Mutuals to include.
    pub mutuals: usize,
}

impl Default for DetailOptions {
    fn default() -> Self {
        DetailOptions {
            interactions: 10,
            mutuals: 10,
        }
    }
}

/// `Person` JSON for each id, in the given order (components without `raw`).
pub fn people(conn: &Connection, cfg: &Config, ids: &[PersonId]) -> Result<Vec<Person>> {
    let _ = (conn, cfg, ids);
    todo!("search stream: person::people")
}

/// `PersonDetail` JSON (`who`, `get_person`). `resolved_from` is the id the
/// caller asked for when it was an absorbed anchor.
pub fn detail(
    conn: &Connection,
    cfg: &Config,
    id: PersonId,
    resolved_from: Option<i64>,
    opts: DetailOptions,
    now: i64,
) -> Result<PersonDetail> {
    let _ = (conn, cfg, id, resolved_from, opts, now);
    todo!("search stream: person::detail")
}
