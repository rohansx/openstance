//! Step 15: person fields from importer hints (by source priority), enrich
//! facts (Phase 3, `egress::facts`) and scalar overrides (LWW; NULL tombstone falls back;
//! privacy flags OR'd per anchor). Spec: `docs/tech-spec/data-model.md`
//! section 4.5 and `entity-resolution.md` section 4.
//!
//! Owned by the "resolve" stream.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::normalize::{name_key, prettify_local_part};
use super::rules::DataFiles;
use crate::config::ResolutionSection;
use crate::error::Result;
use crate::overrides::{self, OverrideField};
use crate::{IdentityId, PersonId};

/// The displayed scalar fields, in `person` column order.
pub const FIELDS: [&str; 5] = ["display_name", "title", "company", "location", "bio"];

/// One participant row of a person with its provenance.
struct HintRow {
    name: Option<String>,
    title: Option<String>,
    company: Option<String>,
    location: Option<String>,
    at: i64,
    interaction_id: i64,
    interaction_kind: String,
    source_kind: String,
    source_id: i64,
    source_label: Option<String>,
}

/// A chosen value and where it came from (`"<kind>:<label>"`).
type Picked = Option<(String, String)>;

/// Priority tier of a hint row for a field; lower wins, `None` = unusable.
fn tier(field: &str, row: &HintRow) -> Option<u8> {
    let src = row.source_kind.as_str();
    match field {
        "display_name" => match src {
            "linkedin_csv" => Some(0),
            "vcf" => Some(1),
            "csv" => Some(2),
            _ if row.interaction_kind == "meeting" => Some(3),
            _ if row.interaction_kind == "email" => Some(4),
            "whatsapp" => Some(5),
            _ => Some(6),
        },
        "title" | "company" => match src {
            "linkedin_csv" => Some(0),
            "vcf" => Some(1),
            "csv" => Some(2),
            _ => Some(3),
        },
        "location" => match src {
            "vcf" => Some(0),
            "csv" => Some(1),
            _ => Some(2),
        },
        _ => None,
    }
}

fn value_of<'a>(field: &str, row: &'a HintRow) -> Option<&'a str> {
    let v = match field {
        "display_name" => row.name.as_deref(),
        "title" => row.title.as_deref(),
        "company" => row.company.as_deref(),
        "location" => row.location.as_deref(),
        _ => None,
    }?;
    let v = v.trim();
    (!v.is_empty()).then_some(v)
}

fn provenance(row: &HintRow) -> String {
    format!(
        "{}:{}",
        row.source_kind,
        row.source_label.as_deref().unwrap_or("")
    )
}

/// The best importer hint for `field` (data-model.md 4.5).
fn pick(field: &str, rows: &[HintRow]) -> Picked {
    let mut best: Option<(u8, &HintRow, &str)> = None;
    // Tier 4 (email display names) is chosen by frequency, not recency.
    let mut email_names: BTreeMap<&str, (u64, &HintRow)> = BTreeMap::new();
    for row in rows {
        let (Some(t), Some(v)) = (tier(field, row), value_of(field, row)) else {
            continue;
        };
        if field == "display_name" && t == 4 {
            if name_key(v).split_whitespace().count() < 2 {
                continue;
            }
            let e = email_names.entry(v).or_insert((0, row));
            e.0 += 1;
            if newer(row, e.1, false) {
                e.1 = row;
            }
            continue;
        }
        let better = match best {
            None => true,
            Some((bt, brow, _)) => t < bt || (t == bt && newer(row, brow, t == 0)),
        };
        if better {
            best = Some((t, row, v));
        }
    }
    if let Some((t, row, v)) = best {
        if t < 4 || email_names.is_empty() {
            return Some((v.to_string(), provenance(row)));
        }
    }
    let freq = email_names.iter().max_by(|(_, (na, ra)), (_, (nb, rb))| {
        na.cmp(nb).then_with(|| {
            if newer(ra, rb, false) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        })
    });
    if let Some((v, (_, row))) = freq {
        return Some((v.to_string(), provenance(row)));
    }
    best.map(|(_, row, v)| (v.to_string(), provenance(row)))
}

/// `a` is preferred over `b` on recency: (latest export first when
/// `by_source`), then latest `at`, then highest interaction id.
fn newer(a: &HintRow, b: &HintRow, by_source: bool) -> bool {
    let ka = (
        if by_source { a.source_id } else { 0 },
        a.at,
        a.interaction_id,
    );
    let kb = (
        if by_source { b.source_id } else { 0 },
        b.at,
        b.interaction_id,
    );
    ka > kb
}

fn hint_rows(conn: &Connection, person: PersonId) -> Result<Vec<HintRow>> {
    let mut st = conn.prepare_cached(
        "SELECT ip.name_hint, ip.title_hint, ip.company_hint, ip.location_hint,
                i.at, i.id, i.kind, s.kind, s.id, s.label
         FROM interaction_person ip
         JOIN interaction i ON i.id = ip.interaction_id
         JOIN source s ON s.id = i.source_id
         WHERE ip.person_id = ?1
         ORDER BY i.id, ip.identity_id, ip.role",
    )?;
    let rows = st
        .query_map([person], |r| {
            Ok(HintRow {
                name: r.get(0)?,
                title: r.get(1)?,
                company: r.get(2)?,
                location: r.get(3)?,
                at: r.get(4)?,
                interaction_id: r.get(5)?,
                interaction_kind: r.get(6)?,
                source_kind: r.get(7)?,
                source_id: r.get(8)?,
                source_label: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn identities(conn: &Connection, person: PersonId) -> Result<Vec<(IdentityId, String, String)>> {
    let mut st = conn.prepare_cached(
        "SELECT i.id, i.kind, i.value FROM identity i WHERE i.person_id = ?1 ORDER BY i.id",
    )?;
    let rows = st
        .query_map([person], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn title_case(key: &str) -> String {
    key.split_whitespace()
        .map(|t| {
            let mut cs = t.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().chain(cs).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Derived (pre-override) value of every displayed field with provenance.
fn derived(
    conn: &Connection,
    data: &DataFiles,
    person: PersonId,
    ids: &[(IdentityId, String, String)],
) -> Result<BTreeMap<&'static str, Picked>> {
    let rows = hint_rows(conn, person)?;
    let mut out = BTreeMap::new();
    for f in FIELDS {
        out.insert(f, pick(f, &rows));
    }
    let emails = || ids.iter().filter(|(_, k, _)| k == "email");
    if out["display_name"].is_none() {
        // whatsapp / name_local, then a prettified email local part.
        let local = ids
            .iter()
            .find(|(_, k, _)| k == "name_local")
            .and_then(|(_, _, v)| v.split_once('|').map(|(_, n)| title_case(n)));
        let pretty = emails()
            .map(|(_, _, v)| prettify_local_part(v))
            .find(|p| !p.is_empty());
        if let Some(n) = local.or(pretty) {
            out.insert("display_name", Some((n, "derived:email".to_string())));
        }
    }
    if out["company"].is_none() {
        let mapped = emails().find_map(|(_, _, v)| {
            let domain = v.rsplit_once('@')?.1;
            let reg = psl::domain_str(domain).unwrap_or(domain);
            data.domains
                .get(domain)
                .or_else(|| data.domains.get(reg))
                .cloned()
        });
        if let Some(k) = mapped {
            out.insert(
                "company",
                Some((title_case(&k), "derived:domain".to_string())),
            );
        }
    }
    Ok(out)
}

fn field_of(name: &str) -> OverrideField {
    match name {
        "display_name" => OverrideField::DisplayName,
        "title" => OverrideField::Title,
        "company" => OverrideField::Company,
        "location" => OverrideField::Location,
        _ => OverrideField::Bio,
    }
}

/// Recompute `display_name, title, company, location, bio, do_not_share,
/// no_enrich` (and `is_bulk` overrides are left to scoring) for `people`, or
/// for everyone when `None`.
pub fn materialize_fields(conn: &Connection, people: Option<&[PersonId]>) -> Result<()> {
    let data = DataFiles::load(&ResolutionSection::default());
    let people: Vec<PersonId> = match people {
        Some(p) => p.to_vec(),
        None => conn
            .prepare("SELECT id FROM person ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?,
    };
    let mut upd = conn.prepare_cached(
        "UPDATE person SET display_name = ?2, title = ?3, company = ?4, location = ?5, bio = ?6,
                do_not_share = ?7, no_enrich = ?8
         WHERE id = ?1",
    )?;
    for p in people {
        let ids = identities(conn, p)?;
        if ids.is_empty() {
            continue;
        }
        let cluster: Vec<IdentityId> = ids.iter().map(|(i, _, _)| *i).collect();
        let base = derived(conn, &data, p, &ids)?;
        // Phase 3: committed enrichment facts rank between overrides and
        // importer hints (enrichment.md section 7).
        let enriched = crate::egress::facts::profile_facts(conn, &cluster)?;
        let mut vals: Vec<Option<String>> = Vec::with_capacity(FIELDS.len());
        for f in FIELDS {
            let v = match overrides::scalar(conn, &cluster, field_of(f))? {
                Some(v) => Some(v),
                None => match enriched.get(f) {
                    Some(fact) if f != "bio" || base[f].is_none() => Some(fact.value.clone()),
                    _ => base[f].as_ref().map(|(v, _)| v.clone()),
                },
            };
            vals.push(v);
        }
        let dns = overrides::privacy_flag(conn, &cluster, OverrideField::DoNotShare)?;
        let ne = overrides::privacy_flag(conn, &cluster, OverrideField::NoEnrich)?;
        upd.execute(rusqlite::params![
            p, vals[0], vals[1], vals[2], vals[3], vals[4], dns, ne
        ])?;
    }
    Ok(())
}

/// Where each displayed scalar of a person came from: `"override"`,
/// `"<source.kind>:<source.label>"`, or `None` (`Person.field_sources`).
/// Values derived without a hint row are `"derived:email"` (prettified
/// local part) and `"derived:domain"` (company from the domain map).
pub fn field_sources(
    conn: &Connection,
    person: PersonId,
) -> Result<BTreeMap<String, Option<String>>> {
    let data = DataFiles::load(&ResolutionSection::default());
    let ids = identities(conn, person)?;
    let cluster: Vec<IdentityId> = if ids.is_empty() {
        vec![person]
    } else {
        ids.iter().map(|(i, _, _)| *i).collect()
    };
    let base = derived(conn, &data, person, &ids)?;
    let enriched = crate::egress::facts::profile_facts(conn, &cluster)?;
    let mut out = BTreeMap::new();
    for f in FIELDS {
        let src = if overrides::scalar(conn, &cluster, field_of(f))?.is_some() {
            Some("override".to_string())
        } else {
            match enriched.get(f) {
                Some(fact) if f != "bio" || base[f].is_none() => Some(fact.provenance.clone()),
                _ => base[f].as_ref().map(|(_, s)| s.clone()),
            }
        };
        out.insert(f.to_string(), src);
    }
    Ok(out)
}
