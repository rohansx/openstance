//! `person_fts` maintenance and bm25 candidates. `rowid = person.id`; any
//! change to a person runs delete + insert in the same transaction; bulk and
//! self people are not indexed. Spec: `docs/tech-spec/data-model.md` 4.10
//! and `search.md` 6.1 (P1-CORE-14).
//!
//! Owned by the "search" stream. Columns: `display_name, title, company,
//! location, bio, tags, handles, subjects` (subjects: distinct, newest first,
//! capped at 200 or 8 KB; notes are never indexed).

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::PersonId;

/// At most this many distinct subjects per person.
pub const MAX_SUBJECTS: usize = 200;
/// At most this many bytes of subjects per person.
pub const MAX_SUBJECT_BYTES: usize = 8 * 1024;

/// bm25 column weights: display_name, title, company, location, bio, tags,
/// handles, subjects (name, title and company over bio and subjects).
const BM25: &str = "bm25(person_fts, 8.0, 5.0, 5.0, 2.0, 1.0, 3.0, 2.0, 1.0)";

/// Split user text into FTS terms: whitespace-separated, lowercased, with
/// non-alphanumeric characters trimmed from both ends; empty terms dropped.
pub fn terms(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// One quoted FTS5 phrase (embedded quotes doubled), with a prefix `*`
/// when the term has 4 or more alphanumeric characters.
pub fn quote_term(term: &str, prefix: bool) -> String {
    let q = format!("\"{}\"", term.replace('"', "\"\""));
    if prefix && term.chars().filter(|c| c.is_alphanumeric()).count() >= 4 {
        format!("{q}*")
    } else {
        q
    }
}

/// Quote every term (doubling embedded quotes) so FTS5 syntax in user input
/// is inert. Terms are OR-ed; 4+ character terms also get a prefix match.
pub fn escape_query(text: &str) -> String {
    terms(text)
        .iter()
        .map(|t| quote_term(t, true))
        .collect::<Vec<_>>()
        .join(" OR ")
}

struct Row {
    display_name: Option<String>,
    title: Option<String>,
    company: Option<String>,
    location: Option<String>,
    bio: Option<String>,
}

fn tags(conn: &Connection, id: PersonId) -> Result<String> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT o.value FROM override o JOIN identity i ON i.id = o.person_id
         WHERE i.person_id = ?1 AND o.field = 'tag' AND o.value IS NOT NULL
         ORDER BY o.value",
    )?;
    let v = stmt
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v.join(" "))
}

fn handles(conn: &Connection, id: PersonId) -> Result<String> {
    let mut stmt = conn.prepare_cached(
        "SELECT kind, value FROM identity
         WHERE person_id = ?1 AND kind IN ('email', 'linkedin_url', 'x_handle', 'ig_handle')
         ORDER BY id",
    )?;
    let mut out: Vec<String> = Vec::new();
    for row in stmt.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (kind, value) = row?;
        if kind == "email" {
            if let Some((_, domain)) = value.rsplit_once('@') {
                let domain = domain.to_string();
                out.push(value);
                if !out.contains(&domain) {
                    out.push(domain);
                }
                continue;
            }
        }
        out.push(value);
    }
    Ok(out.join(" "))
}

fn subjects(conn: &Connection, id: PersonId) -> Result<String> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.subject, max(i.at) AS last FROM interaction_person ip
         JOIN interaction i ON i.id = ip.interaction_id
         WHERE ip.person_id = ?1 AND i.kind IN ('email', 'meeting', 'dm')
           AND i.subject IS NOT NULL AND trim(i.subject) <> ''
         GROUP BY i.subject
         ORDER BY last DESC, i.subject
         LIMIT ?2",
    )?;
    let mut out = String::new();
    let limit = i64::try_from(MAX_SUBJECTS).unwrap_or(i64::MAX);
    for s in stmt.query_map(params![id, limit], |r| r.get::<_, String>(0))? {
        let s = s?;
        let extra = if out.is_empty() { s.len() } else { s.len() + 1 };
        if out.len() + extra > MAX_SUBJECT_BYTES {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&s);
    }
    Ok(out)
}

/// Delete + insert one person's row (removes it for bulk / self / missing).
pub fn reindex_person(conn: &Connection, id: PersonId) -> Result<()> {
    conn.prepare_cached("DELETE FROM person_fts WHERE rowid = ?1")?
        .execute([id])?;
    let row = conn
        .prepare_cached(
            "SELECT display_name, title, company, location, bio FROM person
             WHERE id = ?1 AND is_bulk = 0 AND is_self = 0",
        )?
        .query_row([id], |r| {
            Ok(Row {
                display_name: r.get(0)?,
                title: r.get(1)?,
                company: r.get(2)?,
                location: r.get(3)?,
                bio: r.get(4)?,
            })
        })
        .optional()?;
    let Some(row) = row else {
        return Ok(());
    };
    conn.prepare_cached(
        "INSERT INTO person_fts(rowid, display_name, title, company, location, bio, tags,
                                handles, subjects)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        id,
        row.display_name,
        row.title,
        row.company,
        row.location,
        row.bio,
        tags(conn, id)?,
        handles(conn, id)?,
        subjects(conn, id)?,
    ])?;
    Ok(())
}

/// [`reindex_person`] for several people.
pub fn reindex_people(conn: &Connection, ids: &[PersonId]) -> Result<()> {
    for id in ids {
        reindex_person(conn, *id)?;
    }
    Ok(())
}

/// Delete rows for people that no longer exist (absorbed by a merge).
pub fn remove_people(conn: &Connection, ids: &[PersonId]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM person_fts WHERE rowid = ?1")?;
    for id in ids {
        stmt.execute([id])?;
    }
    Ok(())
}

/// `DELETE FROM person_fts` and reinsert every non-bulk, non-self person.
/// Returns rows written.
pub fn reindex_all(conn: &Connection) -> Result<u64> {
    conn.execute("DELETE FROM person_fts", [])?;
    let ids = conn
        .prepare("SELECT id FROM person WHERE is_bulk = 0 AND is_self = 0 ORDER BY id")?
        .query_map([], |r| r.get::<_, PersonId>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    reindex_people(conn, &ids)?;
    Ok(ids.len() as u64)
}

/// JSON array text for `json_each(?)`.
pub(crate) fn json_ids(ids: &[PersonId]) -> String {
    let parts: Vec<String> = ids.iter().map(i64::to_string).collect();
    format!("[{}]", parts.join(","))
}

/// Top `limit` bm25 hits for `text` (negated bm25, higher is better),
/// restricted to `allowed` when given.
pub fn candidates(
    conn: &Connection,
    text: &str,
    allowed: Option<&[PersonId]>,
    limit: usize,
) -> Result<Vec<(PersonId, f64)>> {
    let m = escape_query(text);
    if m.is_empty() || allowed.is_some_and(<[_]>::is_empty) {
        return Ok(Vec::new());
    }
    // One MATCH ordered by bm25, filtered by `allowed` in Rust. A
    // `rowid IN (...)` constraint makes FTS5 re-run the MATCH once per
    // allowed rowid (seconds at 20k people), so it is never pushed down.
    let sql = format!(
        "SELECT rowid, -{BM25} AS rel FROM person_fts WHERE person_fts MATCH ?1
         ORDER BY rel DESC, rowid ASC"
    );
    let allowed: Option<std::collections::HashSet<PersonId>> =
        allowed.map(|ids| ids.iter().copied().collect());
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params![m])?;
    let mut hits = Vec::new();
    while hits.len() < limit {
        let Some(r) = rows.next()? else { break };
        let id: PersonId = r.get(0)?;
        if allowed.as_ref().is_none_or(|a| a.contains(&id)) {
            hits.push((id, r.get(1)?));
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit::{graph, indexed};

    fn row(c: &Connection, id: PersonId) -> Option<Vec<String>> {
        c.query_row(
            "SELECT display_name, title, company, location, bio, tags, handles, subjects
             FROM person_fts WHERE rowid = ?1",
            [id],
            |r| {
                (0..8)
                    .map(|i| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default))
                    .collect::<rusqlite::Result<Vec<_>>>()
            },
        )
        .ok()
    }

    fn ids(hits: &[(PersonId, f64)]) -> Vec<PersonId> {
        hits.iter().map(|h| h.0).collect()
    }

    #[test]
    fn escape_query_quotes_terms_and_ors_them() {
        assert_eq!(escape_query("ml researchers"), r#""ml" OR "researchers"*"#);
        assert_eq!(escape_query(r#"a"b"#), r#""a""b""#);
        assert_eq!(
            escape_query("stripe OR NOT x*"),
            r#""stripe"* OR "or" OR "not" OR "x""#
        );
        assert_eq!(escape_query("  ) * ( "), "");
        assert_eq!(escape_query(""), "");
    }

    #[test]
    fn reindex_all_indexes_non_bulk_non_self_people() {
        let c = graph();
        assert_eq!(reindex_all(&c).unwrap(), 5);
        assert!(row(&c, 1).is_none(), "self is not indexed");
        assert!(row(&c, 4).is_none(), "bulk is not indexed");
        let n: i64 = c
            .query_row("SELECT count(*) FROM person_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 5);
        // Idempotent.
        assert_eq!(reindex_all(&c).unwrap(), 5);
    }

    #[test]
    fn row_has_fields_tags_handles_and_subjects_newest_first() {
        let c = indexed();
        let r = row(&c, 2).unwrap();
        assert_eq!(r[0], "Priya Raman");
        assert_eq!(r[1], "Co-founder & CEO");
        assert_eq!(r[2], "Northwind");
        assert_eq!(r[3], "San Francisco, CA");
        assert_eq!(
            r[5], "investor-intro",
            "tag written on a non-anchor identity of the cluster"
        );
        assert_eq!(
            r[6],
            "priya@northwind.example northwind.example priya-raman-nw"
        );
        assert_eq!(
            r[7],
            "Northwind offsite Re: seed round notes Northwind x Owen coffee Board deck draft"
        );
        let all: String = (1..=7)
            .filter_map(|i| row(&c, i))
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!all.contains("secret-note-text"), "notes are never indexed");
    }

    #[test]
    fn subjects_are_distinct_and_capped() {
        let c = graph();
        for i in 0..250 {
            c.execute(
                "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
                 VALUES (?1, 1, 'email', ?2, ?3, ?4, 'in')",
                rusqlite::params![
                    1000 + i,
                    i,
                    format!("subject number {}", i % 220),
                    format!("y{i}")
                ],
            )
            .unwrap();
            c.execute(
                "INSERT INTO interaction_person(interaction_id, identity_id, role, person_id)
                 VALUES (?1, 6, 'from', 6)",
                [1000 + i],
            )
            .unwrap();
        }
        reindex_person(&c, 6).unwrap();
        let subjects = &row(&c, 6).unwrap()[7];
        assert!(subjects.starts_with("subject number 29 "), "{subjects}");
        assert_eq!(subjects.matches("subject number").count(), MAX_SUBJECTS);
        assert!(subjects.len() <= MAX_SUBJECT_BYTES);
    }

    #[test]
    fn candidates_rank_by_bm25_and_respect_allowed() {
        let c = indexed();
        let hits = candidates(&c, "devtools", None, 200).unwrap();
        assert_eq!(ids(&hits), [7, 3]);
        assert!(hits[0].1 > hits[1].1, "negated bm25, higher is better");
        assert_eq!(
            ids(&candidates(&c, "devtools", Some(&[3]), 200).unwrap()),
            [3]
        );
        assert!(candidates(&c, "devtools", Some(&[]), 200)
            .unwrap()
            .is_empty());
        assert_eq!(ids(&candidates(&c, "devtools", None, 1).unwrap()), [7]);
        assert_eq!(
            ids(&candidates(&c, "northwind.example", None, 9).unwrap()),
            [2]
        );
        assert_eq!(ids(&candidates(&c, "angels", None, 9).unwrap()), [6]);
    }

    /// Raw `person_fts` rows only (candidates never reads `person`): `n`
    /// people whose subjects all share the term "update".
    fn wide_index(n: i64) -> Connection {
        let c = graph();
        c.execute("DELETE FROM person_fts", []).unwrap();
        let mut stmt = c
            .prepare("INSERT INTO person_fts(rowid, display_name, subjects) VALUES (?1, ?2, ?3)")
            .unwrap();
        for id in 1..=n {
            let subjects: Vec<String> = (0..20)
                .map(|k| format!("Acme update {}", id * 7 + k))
                .collect();
            let name = if id % 10 == 0 {
                "Update Person"
            } else {
                "Some Person"
            };
            stmt.execute(params![id, name, subjects.join(" ")]).unwrap();
        }
        drop(stmt);
        c
    }

    #[test]
    fn restricted_candidates_are_the_allowed_prefix_of_the_global_ranking() {
        let c = wide_index(500);
        let all = candidates(&c, "update", None, 10_000).unwrap();
        assert_eq!(all.len(), 500);
        // Every other id allowed; the limit must apply after the filter.
        let allowed: Vec<PersonId> = (1..=500).filter(|i| i % 2 == 0).collect();
        let got = candidates(&c, "update", Some(&allowed), 30).unwrap();
        let want: Vec<(PersonId, f64)> = all
            .iter()
            .copied()
            .filter(|(id, _)| id % 2 == 0)
            .take(30)
            .collect();
        assert_eq!(got, want);
    }

    /// P1-CORE-16 budget: bm25 top-200 < 10 ms on 20k people. Restricting to
    /// an allowed set must cost about one MATCH, not one MATCH per allowed
    /// rowid (that plan took 5 s on the 20k perf graph).
    #[test]
    fn restricted_candidates_scale_like_one_match() {
        let n = 4000;
        let c = wide_index(n);
        let allowed: Vec<PersonId> = (1..=n).collect();
        let t = std::time::Instant::now();
        let free = candidates(&c, "acme update", None, 200).unwrap();
        let unrestricted = t.elapsed();
        let t = std::time::Instant::now();
        let restricted = candidates(&c, "acme update", Some(&allowed), 200).unwrap();
        let elapsed = t.elapsed();
        assert_eq!(free, restricted);
        assert!(
            elapsed < unrestricted * 5 + std::time::Duration::from_millis(100),
            "restricted {elapsed:?} vs unrestricted {unrestricted:?}"
        );
    }

    #[test]
    fn candidates_treat_fts_syntax_as_plain_text() {
        let c = indexed();
        for text in [
            r#"") OR * NEAR("#,
            "title:ceo",
            "-x",
            "\"",
            "AND",
            "a^b",
            "{x y}:z",
        ] {
            assert!(candidates(&c, text, None, 10).is_ok(), "{text}");
        }
        assert!(candidates(&c, "   ", None, 10).unwrap().is_empty());
    }

    #[test]
    fn reindex_person_reflects_edits_and_drops_bulk_or_missing() {
        let c = indexed();
        c.execute(
            "UPDATE person SET title = 'Founding Engineer' WHERE id = 3",
            [],
        )
        .unwrap();
        assert!(candidates(&c, "founding", None, 10).unwrap().is_empty());
        reindex_person(&c, 3).unwrap();
        assert_eq!(ids(&candidates(&c, "founding", None, 10).unwrap()), [3]);

        c.execute("UPDATE person SET is_bulk = 1 WHERE id = 7", [])
            .unwrap();
        reindex_people(&c, &[7, 3]).unwrap();
        assert!(row(&c, 7).is_none());
        assert!(row(&c, 3).is_some());

        reindex_person(&c, 999).unwrap();
        assert!(row(&c, 999).is_none());
    }

    #[test]
    fn remove_people_deletes_rows() {
        let c = indexed();
        remove_people(&c, &[2, 999]).unwrap();
        assert!(row(&c, 2).is_none());
        assert!(candidates(&c, "priya", None, 10).unwrap().is_empty());
        assert!(row(&c, 3).is_some());
    }
}
