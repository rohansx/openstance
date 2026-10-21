//! Read-only counts over the graph: the numbers `import` prints and the
//! fixture contract in `fixtures/EXPECTED.md` asserts.

use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::json::ResolutionSummary;

/// Row counts and resolution totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GraphCounts {
    /// `source` rows.
    pub sources: u64,
    /// `person` rows, including the self person.
    pub persons: u64,
    /// `person` rows with `is_self = 0`.
    pub persons_excluding_self: u64,
    /// `identity` rows.
    pub identities: u64,
    /// `interaction` rows except `contact`.
    pub interactions: u64,
    /// `contact` interactions (one per vCard / CSV record).
    pub contacts: u64,
    /// People holding at least one identity attached by a flagged rule
    /// (`name_company`, `name_thread`).
    pub flagged_people: u64,
    /// People with `is_bulk = 1`.
    pub bulk: u64,
    /// `tie` rows.
    pub ties: u64,
}

fn count(conn: &Connection, sql: &str) -> Result<u64> {
    let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Compute [`GraphCounts`].
pub fn counts(conn: &Connection) -> Result<GraphCounts> {
    Ok(GraphCounts {
        sources: count(conn, "SELECT count(*) FROM source")?,
        persons: count(conn, "SELECT count(*) FROM person")?,
        persons_excluding_self: count(conn, "SELECT count(*) FROM person WHERE is_self = 0")?,
        identities: count(conn, "SELECT count(*) FROM identity")?,
        interactions: count(
            conn,
            "SELECT count(*) FROM interaction WHERE kind <> 'contact'",
        )?,
        contacts: count(
            conn,
            "SELECT count(*) FROM interaction WHERE kind = 'contact'",
        )?,
        flagged_people: count(conn, FLAGGED_PEOPLE)?,
        bulk: count(conn, "SELECT count(*) FROM person WHERE is_bulk = 1")?,
        ties: count(conn, "SELECT count(*) FROM tie")?,
    })
}

const FLAGGED_PEOPLE: &str = "SELECT count(DISTINCT person_id) FROM identity
     WHERE person_id IS NOT NULL AND match_rule IN ('name_company', 'name_thread')";

/// The `resolution` block of `ImportResponse` (without `people_created`,
/// which the caller computes from before/after counts).
pub fn summary(conn: &Connection) -> Result<ResolutionSummary> {
    Ok(ResolutionSummary {
        people: count(conn, "SELECT count(*) FROM person WHERE is_self = 0")?,
        people_created: 0,
        auto_merged: count(
            conn,
            "SELECT count(*) FROM identity WHERE match_rule IN ('exact', 'gmail_alias', 'override')",
        )?,
        flagged: count(conn, FLAGGED_PEOPLE)?,
        bulk: count(conn, "SELECT count(*) FROM person WHERE is_bulk = 1")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate, open_in_memory};

    fn graph() -> Connection {
        let mut c = open_in_memory().unwrap();
        migrate(&mut c, None, 0).unwrap();
        c.execute_batch(
            "INSERT INTO source(id, kind, imported_at) VALUES (1, 'mbox', 0), (2, 'vcf', 0);
             INSERT INTO person(id, is_self) VALUES (1, 1);
             INSERT INTO person(id) VALUES (2), (4);
             INSERT INTO person(id, is_bulk) VALUES (6, 1);
             INSERT INTO identity(id, kind, value, source_id, person_id, confidence, match_rule) VALUES
               (1, 'email', 'owner@example.com', 1, 1, 1.0, 'anchor'),
               (2, 'email', 'a@example.com', 1, 2, 1.0, 'anchor'),
               (3, 'linkedin_url', 'a-example', 1, 2, 0.85, 'name_company'),
               (4, 'email', 'b@example.com', 1, 4, 1.0, 'anchor'),
               (5, 'email', 'b2@example.com', 1, 4, 0.98, 'gmail_alias'),
               (6, 'email', 'news@example.com', 1, 6, 1.0, 'anchor'),
               (7, 'phone', '+12015550100', 2, 4, 1.0, 'exact');
             INSERT INTO interaction(id, source_id, kind, at, external_id) VALUES
               (1, 1, 'email', 10, 'm1'), (2, 1, 'email', 20, 'm2'), (3, 2, 'contact', 5, 'contact:1');
             INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (2, 4, 'email', 0.3, 10, 1);",
        )
        .unwrap();
        c
    }

    #[test]
    fn counts_cover_every_total_in_expected_md() {
        let c = counts(&graph()).unwrap();
        assert_eq!(
            c,
            GraphCounts {
                sources: 2,
                persons: 4,
                persons_excluding_self: 3,
                identities: 7,
                interactions: 2,
                contacts: 1,
                flagged_people: 1,
                bulk: 1,
                ties: 1,
            }
        );
    }

    #[test]
    fn summary_counts_rule_merges() {
        let s = summary(&graph()).unwrap();
        assert_eq!(s.people, 3);
        assert_eq!(s.flagged, 1);
        assert_eq!(s.bulk, 1);
        assert_eq!(
            s.auto_merged, 2,
            "gmail_alias + exact; anchors and flagged rules excluded"
        );
        assert_eq!(s.people_created, 0);
    }
}
