//! Structured filters to SQL over `person` (step 2). Bound parameters only;
//! user strings are never interpolated. Spec: `docs/tech-spec/search.md`
//! section 5.
//!
//! Owned by the "search" stream.

use openstance_model::Query;
use rusqlite::types::Value;
use rusqlite::Connection;

use super::fts::quote_term;
use super::rules::{location_terms, title_terms, FREEMAIL_LABELS};
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

impl SqlFilter {
    fn bind(&mut self, v: Value) -> String {
        self.params.push(v);
        format!("?{}", self.params.len())
    }

    fn text(&mut self, s: impl Into<String>) -> String {
        self.bind(Value::Text(s.into()))
    }
}

/// Interaction kinds that count as contact (search.md 5.2).
const CONTACT_KINDS: &str = "'email', 'meeting', 'dm', 'connection'";

/// `col : ("a" OR "b"*)` over the given terms.
fn column_match(col: &str, terms: &[String], prefix: bool) -> String {
    let phrases: Vec<String> = terms
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| quote_term(&t, prefix))
        .collect();
    if phrases.is_empty() {
        // Matches nothing, keeps the SQL shape independent of the input.
        return format!("{col} : (\"\u{1}\")");
    }
    format!("{col} : ({})", phrases.join(" OR "))
}

/// LIKE patterns for a company's email domains (`%@stripe.%`, `%.stripe.%`).
/// Personal mail labels never match.
fn domain_patterns(company: &str) -> Vec<String> {
    let lower = company.trim().to_lowercase();
    let words: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    // Always two keys (four patterns) so the SQL shape never depends on input.
    let keys = [words.concat(), words.join("-")];
    let mut out = Vec::new();
    for k in keys {
        if k.is_empty() || FREEMAIL_LABELS.contains(&k.as_str()) {
            out.push("\u{1}".to_string());
            out.push("\u{1}".to_string());
            continue;
        }
        out.push(format!("%@{k}.%"));
        out.push(format!("%.{k}.%"));
    }
    out
}

/// Build the filter for `q` (excludes self, and bulk unless `include_bulk`).
pub fn build(q: &Query) -> SqlFilter {
    let mut f = SqlFilter {
        sql: "p.is_self = 0".to_string(),
        params: Vec::new(),
    };
    if !q.include_bulk {
        f.sql.push_str(" AND p.is_bulk = 0");
    }

    if !q.company.is_empty() {
        let m = f.text(column_match("company", &q.company, false));
        let mut likes = Vec::new();
        for c in &q.company {
            for pat in domain_patterns(c) {
                let ph = f.text(pat);
                likes.push(format!("value LIKE {ph} ESCAPE '\\'"));
            }
        }
        f.sql.push_str(&format!(
            " AND (p.id IN (SELECT rowid FROM person_fts WHERE person_fts MATCH {m})
       OR p.id IN (SELECT person_id FROM identity WHERE kind = 'email' AND ({})))",
            likes.join(" OR ")
        ));
    }
    if !q.title.is_empty() {
        let mut terms: Vec<String> = Vec::new();
        for t in &q.title {
            for x in title_terms(&t.trim().to_lowercase()) {
                if !terms.contains(&x) {
                    terms.push(x);
                }
            }
        }
        let m = f.text(column_match("title", &terms, true));
        f.sql.push_str(&format!(
            " AND p.id IN (SELECT rowid FROM person_fts WHERE person_fts MATCH {m})"
        ));
    }
    if !q.location.is_empty() {
        let mut terms: Vec<String> = Vec::new();
        for l in &q.location {
            for x in location_terms(l) {
                if !terms.contains(&x) {
                    terms.push(x);
                }
            }
        }
        let m = f.text(column_match("location", &terms, false));
        f.sql.push_str(&format!(
            " AND p.id IN (SELECT rowid FROM person_fts WHERE person_fts MATCH {m})"
        ));
    }
    if !q.tags.is_empty() {
        let phs: Vec<String> = q
            .tags
            .iter()
            .map(|t| f.text(t.trim().to_lowercase()))
            .collect();
        f.sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM override o JOIN identity oi ON oi.id = o.person_id
       WHERE oi.person_id = p.id AND o.field = 'tag' AND lower(o.value) IN ({}))",
            phs.join(", ")
        ));
    }

    let window = q.since.is_some() || q.until.is_some();
    if window || q.met_in_person == Some(true) {
        let kinds = if q.met_in_person == Some(true) {
            "'meeting'"
        } else {
            CONTACT_KINDS
        };
        let mut cond = String::new();
        if let Some(s) = q.since {
            let ph = f.bind(Value::Integer(s));
            cond.push_str(&format!(" AND i.at >= {ph}"));
        }
        if let Some(u) = q.until {
            let ph = f.bind(Value::Integer(u));
            cond.push_str(&format!(" AND i.at < {ph}"));
        }
        f.sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM interaction_person ip
       JOIN interaction i ON i.id = ip.interaction_id
       WHERE ip.person_id = p.id AND i.kind IN ({kinds}){cond})"
        ));
    }
    if q.met_in_person == Some(false) {
        f.sql.push_str(
            " AND NOT EXISTS (SELECT 1 FROM interaction_person ip
       JOIN interaction i ON i.id = ip.interaction_id
       WHERE ip.person_id = p.id AND i.kind = 'meeting')",
        );
    }
    if let Some(t) = q.last_contact_before {
        let ph = f.bind(Value::Integer(t));
        f.sql.push_str(&format!(" AND p.last_contact < {ph}"));
    }
    match q.replied {
        Some(true) => f.sql.push_str(" AND p.score_c > 0"),
        Some(false) => f.sql.push_str(" AND p.score_c = 0"),
        None => {}
    }
    if let Some(m) = q.min_score {
        let ph = f.bind(Value::Real(f64::from(m)));
        f.sql.push_str(&format!(" AND p.score >= {ph}"));
    }
    match q.lost_touch {
        Some(true) => f.sql.push_str(" AND p.score_f >= 0.5 AND p.score_r < 0.2"),
        Some(false) => f
            .sql
            .push_str(" AND NOT (p.score_f >= 0.5 AND p.score_r < 0.2)"),
        None => {}
    }
    f
}

/// Structured ordering.
pub const ORDER_BY: &str = "ORDER BY p.score DESC, p.last_contact DESC, p.id ASC";

/// The allowed ids in structured order (`score DESC, last_contact DESC, id ASC`).
pub fn allowed_ids(conn: &Connection, q: &Query) -> Result<Vec<PersonId>> {
    let f = build(q);
    let sql = format!("SELECT p.id FROM person p WHERE {} {ORDER_BY}", f.sql);
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(rusqlite::params_from_iter(f.params.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<PersonId>>>()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit::{ago, indexed};

    fn run(q: Query) -> Vec<PersonId> {
        allowed_ids(&indexed(), &q).unwrap()
    }

    fn with(f: impl FnOnce(&mut Query)) -> Query {
        let mut q = Query::default();
        f(&mut q);
        q
    }

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_filter_excludes_self_and_bulk_in_score_order() {
        assert_eq!(run(Query::default()), [2, 3, 7, 5, 6]);
        assert_eq!(run(with(|q| q.include_bulk = true)), [2, 3, 7, 5, 6, 4]);
    }

    #[test]
    fn company_matches_current_company_or_work_email_domain() {
        assert_eq!(run(with(|q| q.company = v(&["northwind"]))), [2]);
        assert_eq!(
            run(with(|q| q.company = v(&["stripe"]))),
            [5],
            "email domain"
        );
        assert_eq!(run(with(|q| q.company = v(&["contoso"]))), [3, 7]);
        assert_eq!(
            run(with(|q| q.company = v(&["northwind", "halcyon ventures"]))),
            [2, 6],
            "values are OR-ed"
        );
        assert!(run(with(|q| q.company = v(&["gmail"]))).is_empty());
    }

    #[test]
    fn title_and_location_expand_aliases() {
        assert_eq!(run(with(|q| q.title = v(&["founder"]))), [2]);
        assert_eq!(run(with(|q| q.title = v(&["engineer"]))), [3]);
        assert_eq!(run(with(|q| q.title = v(&["pm"]))), [5]);
        assert_eq!(run(with(|q| q.location = v(&["san francisco"]))), [2]);
        assert_eq!(run(with(|q| q.location = v(&["sf"]))), [2]);
        assert_eq!(run(with(|q| q.location = v(&["berlin", "london"]))), [3, 6]);
    }

    #[test]
    fn tags_match_any_identity_of_the_cluster() {
        assert_eq!(run(with(|q| q.tags = v(&["Angels"]))), [6]);
        assert_eq!(run(with(|q| q.tags = v(&["investor-intro"]))), [2]);
    }

    #[test]
    fn interaction_window_and_meetings() {
        assert_eq!(run(with(|q| q.met_in_person = Some(true))), [2, 3, 7]);
        assert_eq!(run(with(|q| q.met_in_person = Some(false))), [5, 6]);
        assert_eq!(
            run(with(|q| {
                q.met_in_person = Some(true);
                q.since = Some(ago(25));
                q.until = Some(ago(0));
            })),
            [3, 7]
        );
        assert_eq!(run(with(|q| q.since = Some(ago(10)))), [2]);
        assert_eq!(run(with(|q| q.until = Some(ago(300)))), [3, 5]);
        assert_eq!(
            run(with(|q| {
                q.since = Some(ago(250));
                q.until = Some(ago(150));
            })),
            [6],
            "connections count as contact"
        );
    }

    #[test]
    fn stored_score_columns() {
        assert_eq!(run(with(|q| q.last_contact_before = Some(ago(100)))), [5]);
        assert_eq!(run(with(|q| q.replied = Some(true))), [2, 7, 5]);
        assert_eq!(run(with(|q| q.replied = Some(false))), [3, 6]);
        assert_eq!(run(with(|q| q.min_score = Some(0.45))), [2, 3, 7]);
        assert_eq!(run(with(|q| q.lost_touch = Some(true))), [5]);
        assert_eq!(run(with(|q| q.lost_touch = Some(false))), [2, 3, 7, 6]);
    }

    #[test]
    fn fields_are_and_ed() {
        assert_eq!(
            run(with(|q| {
                q.company = v(&["contoso"]);
                q.title = v(&["engineer"]);
            })),
            [3]
        );
        assert!(run(with(|q| {
            q.company = v(&["northwind"]);
            q.replied = Some(false);
        }))
        .is_empty());
    }

    #[test]
    fn industry_and_text_never_filter() {
        assert_eq!(
            run(with(|q| {
                q.industry = v(&["fintech"]);
                q.text = Some("nothing matches this".into());
            })),
            [2, 3, 7, 5, 6]
        );
    }

    #[test]
    fn hostile_strings_are_bound_not_interpolated() {
        let evil = [
            r#"") OR 1=1 --"#,
            "' OR '1'='1",
            "*",
            "NEAR(a b)",
            "title:ceo",
            "%",
            "_",
            "\\",
            "\"",
        ];
        for e in evil {
            let q = with(|q| {
                q.company = v(&[e]);
                q.title = v(&[e]);
                q.location = v(&[e]);
                q.tags = v(&[e]);
            });
            let f = build(&q);
            if e.chars().any(char::is_alphanumeric) {
                assert!(!f.sql.contains(e), "{e}: {}", f.sql);
            }
            let benign = with(|q| {
                q.company = v(&["acme"]);
                q.title = v(&["acme"]);
                q.location = v(&["acme"]);
                q.tags = v(&["acme"]);
            });
            assert_eq!(
                f.sql,
                build(&benign).sql,
                "SQL shape never depends on input text"
            );
            assert!(run(q).is_empty(), "{e}");
        }
    }

    #[test]
    fn generated_sql_snapshot() {
        let q = with(|q| {
            q.company = v(&["stripe", "jane street"]);
            q.title = v(&["founder"]);
            q.location = v(&["sf"]);
            q.tags = v(&["angels"]);
            q.since = Some(100);
            q.until = Some(200);
            q.last_contact_before = Some(300);
            q.met_in_person = Some(true);
            q.replied = Some(true);
            q.min_score = Some(0.5);
            q.lost_touch = Some(true);
        });
        let f = build(&q);
        insta::assert_snapshot!(
            "filters_all_fields",
            format!("{}\n-- params --\n{:?}", f.sql, f.params)
        );
    }
}
