//! Union-find over identity ids and resolution steps 1-13.
//! Spec: `docs/tech-spec/entity-resolution.md` section 5 (P1-CORE-07).
//!
//! Owned by the "resolve" stream. `union(a, b)` always makes the smaller
//! root the parent, records `(rule, confidence)` on the absorbed root in a
//! never-compressed union tree, and checks `split_from` cannot-links first.
//! Order: `[me]` and `merged_into` must-links, rule 1 co-reference, rule 2
//! alias keys, rule 3 name+company (with the large-company guard), rule 4
//! name+thread, enrichment identities (Phase 3), dormant filter.

use rusqlite::Connection;

use super::Resolution;
use crate::config::Config;
use crate::error::Result;
use crate::IdentityId;

/// Union-find with a union tree that remembers why each union happened.
#[derive(Debug, Clone, Default)]
pub struct UnionFind {
    #[allow(dead_code)]
    parent: std::collections::BTreeMap<IdentityId, IdentityId>,
}

impl UnionFind {
    /// Root (anchor) of `x`.
    pub fn find(&mut self, x: IdentityId) -> IdentityId {
        let _ = x;
        todo!("resolve stream: UnionFind::find")
    }
}

/// Steps 1-13 over the whole database (read-only).
pub fn resolve(conn: &Connection, cfg: &Config) -> Result<Resolution> {
    let _ = (conn, cfg);
    todo!("resolve stream: P1-CORE-07 resolve (steps 1-13)")
}
