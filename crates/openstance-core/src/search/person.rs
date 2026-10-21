//! Builders for the `Person` and `PersonDetail` JSON shapes
//! (`docs/tech-spec/interfaces.md` 1.2 and 1.8), used by search results,
//! `who` (Phase 2) and export.
//!
//! Owned by the "search" stream.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::json::{
    iso, iso_opt, round3, Identity, MergeInfo, Mutual, NoteJson, OverrideJson, Person,
    PersonDetail, PersonRef, ScoreComponents, SourceRefJson, V,
};
use crate::score::ScoreConfig;
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

/// Scalar fields shown with a provenance in `field_sources`, and the
/// `interaction_person` hint column each one is derived from.
const SOURCED: &[(&str, Option<&str>)] = &[
    ("display_name", Some("name_hint")),
    ("title", Some("title_hint")),
    ("company", Some("company_hint")),
    ("location", Some("location_hint")),
    ("bio", None),
];

struct PersonRow {
    id: PersonId,
    display_name: Option<String>,
    title: Option<String>,
    company: Option<String>,
    location: Option<String>,
    bio: Option<String>,
    first_seen: Option<i64>,
    last_contact: Option<i64>,
    score: f64,
    c: [f64; 5],
    is_bulk: bool,
    do_not_share: bool,
    no_enrich: bool,
}

fn load_row(conn: &Connection, id: PersonId) -> Result<Option<PersonRow>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, display_name, title, company, location, bio, first_seen, last_contact,
                    score, score_r, score_f, score_c, score_m, score_b, is_bulk, do_not_share,
                    no_enrich
             FROM person WHERE id = ?1",
        )?
        .query_row([id], |r| {
            Ok(PersonRow {
                id: r.get(0)?,
                display_name: r.get(1)?,
                title: r.get(2)?,
                company: r.get(3)?,
                location: r.get(4)?,
                bio: r.get(5)?,
                first_seen: r.get(6)?,
                last_contact: r.get(7)?,
                score: r.get(8)?,
                c: [r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?],
                is_bulk: r.get::<_, i64>(14)? != 0,
                do_not_share: r.get::<_, i64>(15)? != 0,
                no_enrich: r.get::<_, i64>(16)? != 0,
            })
        })
        .optional()?)
}

/// Tags of the person's cluster, sorted, distinct.
pub fn tags(conn: &Connection, id: PersonId) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT o.value FROM override o JOIN identity i ON i.id = o.person_id
         WHERE i.person_id = ?1 AND o.field = 'tag' AND o.value IS NOT NULL
         ORDER BY o.value",
    )?;
    let v = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(v)
}

/// The latest override row of a scalar `field` for the cluster:
/// `Some(Some(v))` value, `Some(None)` tombstone, `None` no override.
fn latest_scalar(conn: &Connection, id: PersonId, field: &str) -> Result<Option<Option<String>>> {
    Ok(conn
        .prepare_cached(
            "SELECT o.value FROM override o JOIN identity i ON i.id = o.person_id
             WHERE i.person_id = ?1 AND o.field = ?2
             ORDER BY o.set_at DESC, o.id DESC LIMIT 1",
        )?
        .query_row(params![id, field], |r| r.get::<_, Option<String>>(0))
        .optional()?)
}

/// Where each displayed scalar came from: `override`, `<kind>:<label>` of
/// the most recently imported observation whose hint equals the value (walks
/// the `interaction_person(person_id, interaction_id)` index backwards and
/// stops at the first hit), or `null`. Read-only over
/// raw tables (search never calls resolve).
pub fn field_sources(
    conn: &Connection,
    id: PersonId,
    values: &[(&str, Option<&str>)],
) -> Result<BTreeMap<String, Option<String>>> {
    let mut out = BTreeMap::new();
    let mut enriched = None;
    for (field, hint_col) in SOURCED {
        let value = values
            .iter()
            .find(|(f, _)| f == field)
            .and_then(|(_, v)| *v);
        let src = match (value, latest_scalar(conn, id, field)?) {
            (None, _) => None,
            (Some(_), Some(Some(_))) => Some("override".to_string()),
            (Some(v), _) => match enrich_source(conn, id, &mut enriched, field, v)? {
                Some(src) => Some(src),
                None => match hint_col {
                    Some(col) => {
                        let sql = format!(
                            "SELECT s.kind, s.label FROM interaction_person ip
                         JOIN interaction i ON i.id = ip.interaction_id
                         JOIN source s ON s.id = i.source_id
                         WHERE ip.person_id = ?1 AND ip.{col} = ?2
                         ORDER BY ip.person_id DESC, ip.interaction_id DESC LIMIT 1"
                        );
                        conn.prepare_cached(&sql)?
                            .query_row(params![id, v], |r| {
                                let kind: String = r.get(0)?;
                                let label: Option<String> = r.get(1)?;
                                Ok(match label {
                                    Some(l) => format!("{kind}:{l}"),
                                    None => kind,
                                })
                            })
                            .optional()?
                    }
                    None => None,
                },
            },
        };
        out.insert((*field).to_string(), src);
    }
    Ok(out)
}

/// Provenance of `value` when a committed enrichment fact supplied it
/// (`egress::facts`, loaded once per person).
fn enrich_source(
    conn: &Connection,
    id: PersonId,
    cache: &mut Option<BTreeMap<&'static str, crate::egress::facts::Fact>>,
    field: &str,
    value: &str,
) -> Result<Option<String>> {
    if cache.is_none() {
        let cluster = crate::overrides::cluster(conn, id)?;
        *cache = Some(crate::egress::facts::profile_facts(conn, &cluster)?);
    }
    Ok(cache
        .as_ref()
        .and_then(|m| m.get(field))
        .filter(|f| f.value == value)
        .map(|f| f.provenance.clone()))
}

fn score_config(cfg: &Config) -> Result<ScoreConfig> {
    Ok(ScoreConfig::from_config(&cfg.score)?.0)
}

fn components(row: &PersonRow, sc: &ScoreConfig) -> ScoreComponents {
    ScoreComponents {
        recency: round3(row.c[0]),
        frequency: round3(row.c[1]),
        reciprocity: round3(row.c[2]),
        meetings: round3(row.c[3]),
        breadth: round3(row.c[4]),
        weights: sc.weights.into(),
        preset: sc.preset_name().to_string(),
        half_life_days: sc.half_life_days,
        raw: None,
    }
}

fn build(conn: &Connection, row: PersonRow, sc: &ScoreConfig) -> Result<Person> {
    let fs = field_sources(
        conn,
        row.id,
        &[
            ("display_name", row.display_name.as_deref()),
            ("title", row.title.as_deref()),
            ("company", row.company.as_deref()),
            ("location", row.location.as_deref()),
            ("bio", row.bio.as_deref()),
        ],
    )?;
    let components = components(&row, sc);
    Ok(Person {
        id: row.id,
        tags: tags(conn, row.id)?,
        first_seen: iso_opt(row.first_seen),
        last_contact: iso_opt(row.last_contact),
        score: round3(row.score),
        components,
        is_bulk: row.is_bulk,
        do_not_share: row.do_not_share,
        no_enrich: row.no_enrich,
        field_sources: fs,
        display_name: row.display_name,
        title: row.title,
        company: row.company,
        location: row.location,
        bio: row.bio,
    })
}

/// `Person` JSON for each id, in the given order (components without `raw`).
/// Ids that are not people are skipped.
pub fn people(conn: &Connection, cfg: &Config, ids: &[PersonId]) -> Result<Vec<Person>> {
    let sc = score_config(cfg)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = load_row(conn, *id)? {
            out.push(build(conn, row, &sc)?);
        }
    }
    Ok(out)
}

/// The compact `PersonRef` of a person.
pub fn person_ref(conn: &Connection, id: PersonId) -> Result<Option<PersonRef>> {
    Ok(conn
        .prepare_cached("SELECT id, display_name, title, company, score FROM person WHERE id = ?1")?
        .query_row([id], |r| {
            Ok(PersonRef {
                id: r.get(0)?,
                display_name: r.get(1)?,
                title: r.get(2)?,
                company: r.get(3)?,
                score: round3(r.get(4)?),
            })
        })
        .optional()?)
}

/// Every identity of a person, by id.
pub fn identities(conn: &Connection, id: PersonId) -> Result<Vec<Identity>> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.id, i.kind, i.value, i.confidence, i.match_rule, s.id, s.kind, s.label
         FROM identity i JOIN source s ON s.id = i.source_id
         WHERE i.person_id = ?1 ORDER BY i.id",
    )?;
    let v = stmt
        .query_map([id], |r| {
            Ok(Identity {
                id: r.get(0)?,
                kind: r.get(1)?,
                value: r.get(2)?,
                confidence: r.get(3)?,
                match_rule: r.get(4)?,
                source: SourceRefJson {
                    id: r.get(5)?,
                    kind: r.get(6)?,
                    label: r.get(7)?,
                },
                verification: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

/// Scalar overrides in effect (latest non-tombstone row per field).
pub fn scalar_overrides(conn: &Connection, id: PersonId) -> Result<Vec<OverrideJson>> {
    let mut out = Vec::new();
    for field in [
        "display_name",
        "title",
        "company",
        "location",
        "bio",
        "do_not_share",
        "no_enrich",
        "is_bulk",
    ] {
        let row: Option<(Option<String>, i64)> = conn
            .prepare_cached(
                "SELECT o.value, o.set_at FROM override o JOIN identity i ON i.id = o.person_id
                 WHERE i.person_id = ?1 AND o.field = ?2
                 ORDER BY o.set_at DESC, o.id DESC LIMIT 1",
            )?
            .query_row(params![id, field], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if let Some((Some(value), set_at)) = row {
            out.push(OverrideJson {
                field: field.to_string(),
                value: Some(value),
                set_at: iso(set_at),
            });
        }
    }
    Ok(out)
}

fn notes(conn: &Connection, id: PersonId) -> Result<Vec<NoteJson>> {
    let mut stmt = conn.prepare_cached(
        "SELECT o.value, o.set_at FROM override o JOIN identity i ON i.id = o.person_id
         WHERE i.person_id = ?1 AND o.field = 'note' AND o.value IS NOT NULL
         ORDER BY o.set_at DESC, o.id DESC",
    )?;
    let v = stmt
        .query_map([id], |r| {
            Ok(NoteJson {
                value: r.get(0)?,
                set_at: iso(r.get(1)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

fn mutuals(conn: &Connection, id: PersonId, n: usize) -> Result<Vec<Mutual>> {
    let mut stmt = conn.prepare_cached(
        "SELECT CASE WHEN a = ?1 THEN b ELSE a END AS other, kind, n_shared, weight
         FROM tie WHERE a = ?1 OR b = ?1
         ORDER BY weight DESC, last_at DESC, other ASC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![id, i64::try_from(n).unwrap_or(i64::MAX)], |r| {
            Ok((
                r.get::<_, PersonId>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (other, kind, n_shared, weight) in rows {
        if let Some(p) = person_ref(conn, other)? {
            out.push(Mutual {
                person: p,
                tie_kind: kind,
                n_shared,
                weight: round3(weight),
            });
        }
    }
    Ok(out)
}

fn merge_info(conn: &Connection, id: PersonId, idents: &[Identity]) -> Result<MergeInfo> {
    let cluster: Vec<i64> = idents.iter().map(|i| i.id).collect();
    let mut merged_from = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT person_id, value FROM override WHERE field = 'merged_into' ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (pid, value) in rows {
        let target = value.and_then(|v| v.trim().parse::<i64>().ok());
        for x in [Some(pid), target].into_iter().flatten() {
            if x != id && cluster.contains(&x) && !merged_from.contains(&x) {
                merged_from.push(x);
            }
        }
    }
    merged_from.sort_unstable();
    let flagged = idents.iter().any(|i| {
        matches!(
            i.match_rule.as_deref(),
            Some("name_company") | Some("name_thread")
        )
    });
    let confidence = idents
        .iter()
        .filter_map(|i| i.confidence)
        .reduce(f64::min)
        .unwrap_or(1.0);
    Ok(MergeInfo {
        merged_from,
        flagged,
        confidence,
    })
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
    let sc = score_config(cfg)?;
    let row = load_row(conn, id)?.ok_or_else(|| Error::NotFound(format!("person {id}")))?;
    let mut person = build(conn, row, &sc)?;
    if let Ok(full) = crate::score::detail(conn, &sc, id, now) {
        person.components.raw = full.raw;
    }
    let idents = identities(conn, id)?;
    let recent = super::evidence::recent(conn, &[id], opts.interactions, now)?
        .remove(&id)
        .unwrap_or_default();
    Ok(PersonDetail {
        v: V,
        resolved_from,
        merge: merge_info(conn, id, &idents)?,
        identities: idents,
        recent_interactions: recent,
        mutuals: mutuals(conn, id, opts.mutuals)?,
        overrides: scalar_overrides(conn, id)?,
        notes: notes(conn, id)?,
        suggested: BTreeMap::new(),
        person,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::iso;
    use crate::search::testkit::{ago, indexed, NOW};

    #[test]
    fn people_in_the_given_order_with_components_from_stored_columns() {
        let c = indexed();
        let cfg = Config::default();
        let ps = people(&c, &cfg, &[3, 2, 404]).unwrap();
        assert_eq!(ps.iter().map(|p| p.id).collect::<Vec<_>>(), [3, 2]);
        let p = &ps[1];
        assert_eq!(p.display_name.as_deref(), Some("Priya Raman"));
        assert_eq!(p.title.as_deref(), Some("Co-founder & CEO"));
        assert_eq!(p.tags, ["investor-intro"]);
        assert_eq!(p.last_contact, Some(iso(ago(5))));
        assert_eq!(p.score, 0.8);
        assert_eq!(p.components.recency, 0.95);
        assert_eq!(p.components.breadth, 0.5);
        assert_eq!(p.components.weights.recency, 0.35);
        assert_eq!(p.components.preset, "default");
        assert_eq!(p.components.half_life_days, 180.0);
        assert!(p.components.raw.is_none());
        assert!(!p.is_bulk && !p.do_not_share && !p.no_enrich);
        assert_eq!(
            p.field_sources
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["bio", "company", "display_name", "location", "title"]
        );
    }

    #[test]
    fn field_sources_name_overrides_and_hint_sources() {
        let c = indexed();
        c.execute_batch(
            "INSERT INTO override(person_id, field, value, set_at) VALUES
               (3, 'title', 'Staff Engineer', 10),
               (3, 'company', 'Old Co', 5), (3, 'company', NULL, 6);
             UPDATE interaction_person SET company_hint = 'Contoso', name_hint = 'Marcus Lee'
               WHERE interaction_id = 109 AND identity_id = 3;",
        )
        .unwrap();
        let p = &people(&c, &Config::default(), &[3]).unwrap()[0];
        assert_eq!(p.field_sources["title"].as_deref(), Some("override"));
        assert_eq!(
            p.field_sources["company"].as_deref(),
            Some("mbox:work gmail"),
            "a tombstone falls back to the derived value's source"
        );
        assert_eq!(
            p.field_sources["display_name"].as_deref(),
            Some("mbox:work gmail")
        );
        assert_eq!(p.field_sources["bio"], None);
    }

    #[test]
    fn detail_has_identities_interactions_mutuals_overrides_and_notes() {
        let c = indexed();
        c.execute_batch(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (2, 3, 'email', 0.3, 0, 1), (3, 7, 'meeting', 0.5, 0, 1);
             INSERT INTO override(person_id, field, value, set_at) VALUES
               (3, 'title', 'Principal Engineer', 10),
               (3, 'note', 'later note', 20),
               (3, 'merged_into', '3', 30);
             UPDATE identity SET person_id = 3, match_rule = 'name_thread', confidence = 0.75
               WHERE id = 9;",
        )
        .unwrap();
        let d = detail(
            &c,
            &Config::default(),
            3,
            Some(9),
            DetailOptions::default(),
            NOW,
        )
        .unwrap();
        assert_eq!(d.v, 1);
        assert_eq!(d.resolved_from, Some(9));
        assert_eq!(d.person.id, 3);
        assert_eq!(
            d.identities.iter().map(|i| i.id).collect::<Vec<_>>(),
            [3, 9]
        );
        assert_eq!(d.identities[1].match_rule.as_deref(), Some("name_thread"));
        assert_eq!(d.identities[0].source.label.as_deref(), Some("work gmail"));
        assert_eq!(d.recent_interactions.len(), 3);
        assert_eq!(
            d.mutuals.iter().map(|m| m.person.id).collect::<Vec<_>>(),
            [7, 2],
            "strongest tie first"
        );
        assert_eq!(d.mutuals[0].tie_kind, "meeting");
        assert_eq!(d.overrides.len(), 1);
        assert_eq!(d.overrides[0].field, "title");
        assert_eq!(d.overrides[0].value.as_deref(), Some("Principal Engineer"));
        assert_eq!(
            d.notes.iter().map(|n| n.value.as_str()).collect::<Vec<_>>(),
            ["later note", "secret-note-text"]
        );
        assert!(d.merge.flagged);
        assert_eq!(d.merge.confidence, 0.75);
        assert!(d.suggested.is_empty());
    }

    #[test]
    fn detail_of_missing_person_is_not_found() {
        let c = indexed();
        let e = detail(
            &c,
            &Config::default(),
            404,
            None,
            DetailOptions::default(),
            NOW,
        )
        .unwrap_err();
        assert!(matches!(e, crate::Error::NotFound(_)));
    }
}
