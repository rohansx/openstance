//! Enrichment facts (P3-CORE-02): the person fields that committed
//! enrichment results contribute, replayed from `enrich_log` on every
//! (re)build. Spec: `docs/tech-spec/enrichment.md` section 7.
//!
//! A row counts once it is `ok`, is a `profile` answer, and is attached to an
//! `enrich:<backend>` source (`source_id`, set by `EgressGate::commit`);
//! quoted, blocked, failed or uncommitted rows never become facts. The
//! newest committed value per field wins. Precedence over the other layers
//! is applied by the callers: override > enrich facts > importer hints.

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::enrich::EnrichData;
use crate::error::Result;
use crate::IdentityId;

/// One field value from enrichment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// The value.
    pub value: String,
    /// `"<source.kind>:<source.label>"`, e.g. `enrich:treg:refresh 01J...`.
    pub provenance: String,
    /// The `enrich_log` row it came from.
    pub enrich_log_id: i64,
    /// When it was fetched (unix seconds).
    pub at: i64,
}

/// The displayed fields enrichment can fill. `bio` comes from the
/// profile headline and is only used while no other layer sets a bio.
pub const FACT_FIELDS: [&str; 4] = ["title", "company", "location", "bio"];

/// The newest committed enrichment value of each field for a cluster of
/// identity ids (`enrich_log.person_id` is an anchor identity id).
pub fn profile_facts(
    conn: &Connection,
    cluster: &[IdentityId],
) -> Result<BTreeMap<&'static str, Fact>> {
    let mut out = BTreeMap::new();
    if cluster.is_empty() {
        return Ok(out);
    }
    let mut st = conn.prepare_cached(
        "SELECT e.id, e.at, e.response_json, s.kind, s.label
         FROM enrich_log e JOIN source s ON s.id = e.source_id
         WHERE e.person_id = ?1 AND e.status = 'ok' AND e.op = 'profile'
           AND e.response_json IS NOT NULL",
    )?;
    let mut rows: Vec<(i64, i64, String, String)> = Vec::new();
    for &ident in cluster {
        let found = st.query_map([ident], |r| {
            let kind: String = r.get(3)?;
            let label: Option<String> = r.get(4)?;
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                match label {
                    Some(l) => format!("{kind}:{l}"),
                    None => kind,
                },
            ))
        })?;
        for row in found {
            rows.push(row?);
        }
    }
    // Newest first: send time, then row id.
    rows.sort_by_key(|r| std::cmp::Reverse((r.1, r.0)));
    for (log_id, at, json, provenance) in rows {
        let Ok(EnrichData::Profile(p)) = serde_json::from_str::<EnrichData>(&json) else {
            continue;
        };
        let values = [
            ("title", p.title),
            ("company", p.company),
            ("location", p.location),
            ("bio", p.headline),
        ];
        for (field, v) in values {
            let Some(v) = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else {
                continue;
            };
            out.entry(field).or_insert_with(|| Fact {
                value: v,
                provenance: provenance.clone(),
                enrich_log_id: log_id,
                at,
            });
        }
    }
    Ok(out)
}
