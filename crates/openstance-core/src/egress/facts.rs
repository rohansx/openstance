//! Enrichment facts (P3-CORE-02): the person fields that committed
//! enrichment results contribute, replayed from `enrich_log` on every
//! (re)build. Spec: `docs/tech-spec/enrichment.md` section 7.
//!
//! A row counts once it is `ok`, is a `profile` answer, and is attached to an
//! `enrich:<backend>` source (`source_id`, set by `EgressGate::commit`);
//! quoted, blocked, failed or uncommitted rows never become facts. The
//! newest committed value per field wins. Precedence over the other layers
//! is applied by the callers: override > enrich facts > importer hints.
//!
//! The same committed rows also carry identities (found emails, LinkedIn
//! URLs; [`identity_claims`]), which resolution step 12 attaches to the
//! person, and email verification verdicts ([`verifications`]), which
//! `get_person` shows on the matching email identity.

use std::collections::BTreeMap;

use openstance_import::HandleKind;
use rusqlite::Connection;

use crate::enrich::{EnrichData, VerifyStatus};
use crate::error::Result;
use crate::resolve::normalize::{normalize_handle, NormalizeContext};
use crate::IdentityId;

/// Every enrichment edge is capped at this confidence (data-model.md 4.2).
pub const ENRICH_CONFIDENCE_CAP: f64 = 0.9;

/// Confidence of a LinkedIn URL from a profile answer, before the cap.
const PROFILE_LINKEDIN_CONFIDENCE: f64 = 0.95;

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

/// A handle an enrichment result says belongs to a person.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityClaim {
    /// The `enrich_log` row it came from.
    pub enrich_log_id: i64,
    /// `enrich_log.person_id` (an anchor identity id).
    pub person: IdentityId,
    /// Handle kind (`email` or `linkedin_url`).
    pub kind: HandleKind,
    /// Normalized value, as importers store it.
    pub value: String,
    /// Edge confidence, already capped at [`ENRICH_CONFIDENCE_CAP`].
    pub confidence: f64,
}

/// The normalized handles one result carries, with their capped confidence.
pub fn handles_of(data: &EnrichData) -> Vec<(HandleKind, String, f64)> {
    let ctx = NormalizeContext::default();
    let (kind, raw, conf) = match data {
        EnrichData::Email { email, confidence } => {
            (HandleKind::Email, email.as_str(), f64::from(*confidence))
        }
        EnrichData::Profile(p) => match p.linkedin.as_deref() {
            Some(l) => (HandleKind::LinkedinUrl, l, PROFILE_LINKEDIN_CONFIDENCE),
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    let Some(value) = normalize_handle(kind, raw, &ctx) else {
        return Vec::new();
    };
    let conf = if conf.is_finite() { conf } else { 0.0 };
    // f32 -> f64 widening is not exact; round to 6 places.
    let conf = (conf.clamp(0.0, ENRICH_CONFIDENCE_CAP) * 1e6).round() / 1e6;
    vec![(kind, value, conf)]
}

/// Every committed identity claim (`ok` `profile` / `find_email` rows with a
/// source and a person), in `enrich_log.id` order.
pub fn identity_claims(conn: &Connection) -> Result<Vec<IdentityClaim>> {
    let mut st = conn.prepare_cached(
        "SELECT id, person_id, response_json FROM enrich_log
         WHERE status = 'ok' AND op IN ('profile', 'find_email')
           AND source_id IS NOT NULL AND person_id IS NOT NULL
           AND response_json IS NOT NULL
         ORDER BY id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (log_id, person, json) in rows {
        let Ok(data) = serde_json::from_str::<EnrichData>(&json) else {
            continue;
        };
        for (kind, value, confidence) in handles_of(&data) {
            out.push(IdentityClaim {
                enrich_log_id: log_id,
                person,
                kind,
                value,
                confidence,
            });
        }
    }
    Ok(out)
}

/// A committed verification verdict for one email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationFact {
    /// The verdict.
    pub status: VerifyStatus,
    /// When it was fetched (`enrich_log.at`, unix seconds).
    pub at: i64,
    /// The source kind, e.g. `enrich:treg`.
    pub via: String,
    /// The `enrich_log` row.
    pub enrich_log_id: i64,
}

/// The newest committed verification verdict of each of `emails`
/// (normalized values), keyed by that value.
pub fn verifications(
    conn: &Connection,
    emails: &[&str],
) -> Result<BTreeMap<String, VerificationFact>> {
    let mut out = BTreeMap::new();
    if emails.is_empty() {
        return Ok(out);
    }
    let mut st = conn.prepare_cached(
        "SELECT e.id, e.at, e.response_json, s.kind
         FROM enrich_log e JOIN source s ON s.id = e.source_id
         WHERE e.status = 'ok' AND e.op = 'verify_email' AND e.response_json IS NOT NULL
         ORDER BY e.at DESC, e.id DESC",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let ctx = NormalizeContext::default();
    for (log_id, at, json, via) in rows {
        let Ok(EnrichData::Verification { email, status }) = serde_json::from_str(&json) else {
            continue;
        };
        let Some(email) = normalize_handle(HandleKind::Email, &email, &ctx) else {
            continue;
        };
        if emails.contains(&email.as_str()) {
            out.entry(email).or_insert(VerificationFact {
                status,
                at,
                via,
                enrich_log_id: log_id,
            });
        }
    }
    Ok(out)
}
