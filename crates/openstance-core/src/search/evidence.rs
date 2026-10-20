//! Evidence for results: the N most recent contact interactions per person
//! (one windowed query per page) and the rendered evidence line
//! `{YYYY-MM-DD}  {kind} {direction}  "{subject}"  ({source.label})`.
//! Spec: `docs/tech-spec/search.md` section 8.
//!
//! Owned by the "search" stream. Subjects only, never bodies.

use std::collections::BTreeMap;

use openstance_model::Query;
use rusqlite::{params, Connection};
use serde_json::json;

use super::filters::domain_patterns;
use super::fts::json_ids;
use super::rules::{company_norm, location_terms, title_terms};
use crate::error::Result;
use crate::json::{iso, ymd, Evidence, Matched, Person, SourceRefJson};
use crate::PersonId;

/// Up to `n` most recent `email/meeting/dm/connection` interactions with
/// `at <= now` per person.
pub fn recent(
    conn: &Connection,
    people: &[PersonId],
    n: usize,
    now: i64,
) -> Result<BTreeMap<PersonId, Vec<Evidence>>> {
    let mut out: BTreeMap<PersonId, Vec<Evidence>> = BTreeMap::new();
    if n == 0 || people.is_empty() {
        return Ok(out);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT person_id, interaction_id, kind, direction, at, subject, role,
                source_id, source_kind, source_label FROM (
           SELECT ip.person_id, i.id AS interaction_id, i.kind, i.direction, i.at, i.subject,
                  min(ip.role) AS role, s.id AS source_id, s.kind AS source_kind,
                  s.label AS source_label,
                  row_number() OVER (PARTITION BY ip.person_id ORDER BY i.at DESC, i.id DESC) AS rn
           FROM interaction_person ip
           JOIN interaction i ON i.id = ip.interaction_id
           JOIN source s ON s.id = i.source_id
           WHERE ip.person_id IN (SELECT value FROM json_each(?1))
             AND i.kind IN ('email', 'meeting', 'dm', 'connection')
             AND i.at <= ?2
           GROUP BY ip.person_id, i.id
         )
         WHERE rn <= ?3
         ORDER BY person_id, rn",
    )?;
    let limit = i64::try_from(n).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![json_ids(people), now, limit], |r| {
        let person: PersonId = r.get(0)?;
        let at: i64 = r.get(4)?;
        let kind: String = r.get(2)?;
        let direction: Option<String> = r.get(3)?;
        let subject: Option<String> = r.get(5)?;
        let label: Option<String> = r.get(9)?;
        let l = line(
            at,
            &kind,
            direction.as_deref(),
            subject.as_deref(),
            label.as_deref(),
        );
        Ok((
            person,
            Evidence::Interaction {
                interaction_id: r.get(1)?,
                kind,
                direction,
                at: iso(at),
                subject,
                role: r.get(6)?,
                source: SourceRefJson {
                    id: r.get(7)?,
                    kind: r.get(8)?,
                    label,
                },
                line: l,
            },
        ))
    })?;
    for row in rows {
        let (p, e) = row?;
        out.entry(p).or_default().push(e);
    }
    Ok(out)
}

/// Render one evidence line.
pub fn line(
    at: i64,
    kind: &str,
    direction: Option<&str>,
    subject: Option<&str>,
    source_label: Option<&str>,
) -> String {
    let mut s = format!("{}  {kind}", ymd(at));
    if let Some(d) = direction.filter(|d| !d.is_empty() && *d != "none") {
        s.push(' ');
        s.push_str(d);
    }
    let subject = match subject.map(str::trim).filter(|x| !x.is_empty()) {
        Some(x) => x.to_string(),
        None => match kind {
            "dm" => "message".to_string(),
            "connection" => "connected".to_string(),
            _ => "(no subject)".to_string(),
        },
    };
    s.push_str(&format!("  \"{subject}\""));
    if let Some(l) = source_label.filter(|l| !l.is_empty()) {
        s.push_str(&format!("  ({l})"));
    }
    s
}

fn contains_word(hay: &str, needle: &str) -> bool {
    let hay = format!(" {} ", company_norm(hay));
    let needle = company_norm(needle);
    !needle.is_empty() && hay.contains(&format!(" {needle} "))
}

/// Matched-field entries for a person against the parsed query.
pub fn matched_fields(q: &Query, person: &Person) -> Vec<Matched> {
    let mut out = Vec::new();
    if let Some(company) = &person.company {
        for c in &q.company {
            if contains_word(company, c) {
                out.push(Matched {
                    field: "company".into(),
                    value: Some(json!(company)),
                    matched: Some(c.to_lowercase()),
                    via: Some("person.company".into()),
                    ..Matched::default()
                });
                break;
            }
        }
    }
    if let Some(title) = &person.title {
        let lower = title.to_lowercase();
        'title: for t in &q.title {
            for term in title_terms(&t.to_lowercase()) {
                if lower.contains(&term) {
                    out.push(Matched {
                        field: "title".into(),
                        value: Some(json!(title)),
                        matched: Some(t.to_lowercase()),
                        ..Matched::default()
                    });
                    break 'title;
                }
            }
        }
    }
    if let Some(loc) = &person.location {
        let lower = format!(" {} ", super::rules::simple_norm(loc));
        'loc: for l in &q.location {
            let terms = location_terms(l);
            for term in &terms {
                if lower.contains(&format!(" {term} ")) {
                    out.push(Matched {
                        field: "location".into(),
                        value: Some(json!(loc)),
                        matched: terms.first().cloned(),
                        ..Matched::default()
                    });
                    break 'loc;
                }
            }
        }
    }
    for t in &q.tags {
        let t = t.trim().to_lowercase();
        if person.tags.iter().any(|pt| pt.to_lowercase() == t) {
            out.push(Matched {
                field: "tags".into(),
                value: Some(json!(t)),
                matched: Some(t.clone()),
                ..Matched::default()
            });
        }
    }
    for ind in &q.industry {
        let ind = ind.trim().to_lowercase();
        if ind.is_empty() {
            continue;
        }
        let tag = person.tags.iter().find(|t| t.to_lowercase().contains(&ind));
        let hit = if let Some(t) = tag {
            Some((json!(t), "tag"))
        } else {
            [
                ("title", &person.title),
                ("company", &person.company),
                ("bio", &person.bio),
            ]
            .into_iter()
            .find_map(|(name, v)| {
                v.as_ref()
                    .filter(|v| v.to_lowercase().contains(&ind))
                    .map(|v| (json!(v), name))
            })
        };
        if let Some((value, via)) = hit {
            out.push(Matched {
                field: "industry".into(),
                value: Some(value),
                matched: Some(ind.clone()),
                via: Some(via.into()),
                ..Matched::default()
            });
        }
    }
    if let Some(m) = q.met_in_person {
        out.push(Matched {
            field: "met_in_person".into(),
            value: Some(json!(m)),
            ..Matched::default()
        });
    }
    let c = &person.components;
    if let Some(r) = q.replied {
        out.push(Matched {
            field: "replied".into(),
            value: Some(json!(r)),
            via: Some(format!("reciprocity {:.2}", c.reciprocity)),
            ..Matched::default()
        });
    }
    if let Some(min) = q.min_score {
        out.push(Matched {
            field: "min_score".into(),
            value: Some(json!(person.score)),
            via: Some(format!("score {:.2} >= {:.2}", person.score, min)),
            ..Matched::default()
        });
    }
    if let Some(l) = q.lost_touch {
        out.push(Matched {
            field: "lost_touch".into(),
            value: Some(json!(l)),
            via: Some(format!(
                "frequency {:.2}, recency {:.2}",
                c.frequency, c.recency
            )),
            ..Matched::default()
        });
    }
    out
}

/// `company` entries for companies matched only through a work email
/// domain (`"via": "email domain stripe.example"`).
pub fn company_domain_matches(
    conn: &Connection,
    person: PersonId,
    companies: &[String],
) -> Result<Vec<Matched>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT value FROM identity
         WHERE person_id = ?1 AND kind = 'email' AND (value LIKE ?2 ESCAPE '\\' OR value LIKE ?3 ESCAPE '\\')
         ORDER BY id LIMIT 1",
    )?;
    for c in companies {
        let pats = domain_patterns(c);
        for pair in pats.chunks(2) {
            let [a, b] = pair else { continue };
            let hit: Option<String> = stmt
                .query_map(params![person, a, b], |r| r.get(0))?
                .next()
                .transpose()?;
            if let Some(email) = hit {
                let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_string();
                out.push(Matched {
                    field: "company".into(),
                    matched: Some(c.to_lowercase()),
                    via: Some(format!("email domain {domain}")),
                    ..Matched::default()
                });
                break;
            }
        }
    }
    Ok(out)
}

/// Meeting count for the `met_in_person` matched entry (`"4 meetings"`).
pub fn meetings_via(conn: &Connection, person: PersonId, now: i64) -> Result<String> {
    let n: i64 = conn
        .prepare_cached(
            "SELECT count(DISTINCT i.id) FROM interaction_person ip
             JOIN interaction i ON i.id = ip.interaction_id
             WHERE ip.person_id = ?1 AND i.kind = 'meeting' AND i.at <= ?2",
        )?
        .query_row(params![person, now], |r| r.get(0))?;
    Ok(if n == 1 {
        "1 meeting".to_string()
    } else {
        format!("{n} meetings")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::iso;
    use crate::search::testkit::{ago, indexed, NOW};

    fn ids(v: &[Evidence]) -> Vec<i64> {
        v.iter()
            .map(|e| match e {
                Evidence::Interaction { interaction_id, .. } => *interaction_id,
                _ => -1,
            })
            .collect()
    }

    #[test]
    fn line_format_matches_the_spec() {
        let at = ago(5);
        let day = crate::json::ymd(at);
        assert_eq!(
            line(
                at,
                "email",
                Some("in"),
                Some("Re: seed round notes"),
                Some("work gmail")
            ),
            format!("{day}  email in  \"Re: seed round notes\"  (work gmail)")
        );
        assert_eq!(
            line(
                at,
                "meeting",
                Some("mutual"),
                Some("devtools dinner"),
                Some("work calendar")
            ),
            format!("{day}  meeting mutual  \"devtools dinner\"  (work calendar)")
        );
        assert_eq!(
            line(at, "dm", Some("out"), None, Some("linkedin 2026-10")),
            format!("{day}  dm out  \"message\"  (linkedin 2026-10)")
        );
        assert_eq!(
            line(at, "connection", Some("none"), None, None),
            format!("{day}  connection  \"connected\"")
        );
    }

    #[test]
    fn recent_returns_newest_contact_interactions_up_to_now() {
        let c = indexed();
        let got = recent(&c, &[2, 3, 6, 99], 3, NOW).unwrap();
        assert_eq!(
            ids(&got[&2]),
            [101, 102, 103],
            "future meeting 110 excluded"
        );
        assert_eq!(ids(&got[&3]), [107, 103, 109]);
        assert_eq!(ids(&got[&6]), [106]);
        assert!(!got.contains_key(&99));
        match &got[&2][0] {
            Evidence::Interaction {
                kind,
                direction,
                at,
                subject,
                role,
                source,
                line: l,
                ..
            } => {
                assert_eq!(kind, "email");
                assert_eq!(direction.as_deref(), Some("in"));
                assert_eq!(at, &iso(ago(5)));
                assert_eq!(subject.as_deref(), Some("Re: seed round notes"));
                assert_eq!(role.as_deref(), Some("from"));
                assert_eq!(source.label.as_deref(), Some("work gmail"));
                assert_eq!(source.kind, "mbox");
                assert!(l.ends_with("\"Re: seed round notes\"  (work gmail)"), "{l}");
            }
            other => panic!("{other:?}"),
        }
        let one = recent(&c, &[2], 1, NOW).unwrap();
        assert_eq!(ids(&one[&2]), [101]);
        assert!(recent(&c, &[2], 0, NOW).unwrap().is_empty());
        assert!(recent(&c, &[], 3, NOW).unwrap().is_empty());
    }

    #[test]
    fn matched_fields_explain_structured_hits() {
        let c = indexed();
        let cfg = crate::config::Config::default();
        let people = crate::search::person::people(&c, &cfg, &[2, 5, 6]).unwrap();
        let q = Query {
            company: vec!["northwind".into()],
            title: vec!["founder".into()],
            location: vec!["sf".into()],
            tags: vec!["investor-intro".into()],
            lost_touch: Some(true),
            replied: Some(true),
            min_score: Some(0.4),
            ..Query::default()
        };
        let m = matched_fields(&q, &people[0]);
        let fields: Vec<&str> = m.iter().map(|x| x.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "company",
                "title",
                "location",
                "tags",
                "replied",
                "min_score",
                "lost_touch"
            ]
        );
        assert_eq!(m[0].value, Some(serde_json::json!("Northwind")));
        assert_eq!(m[0].matched.as_deref(), Some("northwind"));
        assert_eq!(m[0].via.as_deref(), Some("person.company"));
        assert_eq!(m[1].value, Some(serde_json::json!("Co-founder & CEO")));
        assert_eq!(m[1].matched.as_deref(), Some("founder"));
        assert_eq!(m[2].value, Some(serde_json::json!("San Francisco, CA")));
        assert_eq!(m[2].matched.as_deref(), Some("san francisco"));
        assert_eq!(m[6].via.as_deref(), Some("frequency 0.90, recency 0.95"));

        // Sam: company only via email domain -> no person.company entry here.
        let q = Query {
            company: vec!["stripe".into()],
            industry: vec!["fintech".into()],
            ..Query::default()
        };
        assert!(matched_fields(&q, &people[1]).is_empty());
        // Lena: industry matched in the bio.
        let m = matched_fields(&q, &people[2]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].field, "industry");
        assert_eq!(m[0].matched.as_deref(), Some("fintech"));
        assert_eq!(m[0].via.as_deref(), Some("bio"));
    }

    #[test]
    fn domain_matches_say_via_email_domain() {
        let c = indexed();
        let m = company_domain_matches(&c, 5, &["stripe".to_string(), "northwind".to_string()])
            .unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].field, "company");
        assert_eq!(m[0].matched.as_deref(), Some("stripe"));
        assert_eq!(m[0].via.as_deref(), Some("email domain stripe.example"));
    }
}
