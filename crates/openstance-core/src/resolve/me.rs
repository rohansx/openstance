//! Who "me" is when `[me] identities` is empty (configuration.md `[me]`,
//! ingestion.md 6.1/6.2/6.5, entity-resolution.md section 5).
//!
//! Every import reports `self_candidates`. While the config names no self
//! handle, an import whose candidates are unambiguous adopts them:
//!
//! | Importer | Adopted when |
//! |---|---|
//! | `mbox` | the candidate is the `From` of at least [`SENT_SHARE`] of this source's `Sent`-labelled mail |
//! | `linkedin_csv` | exactly one owner profile shows up across SENT messages and invitations |
//! | `x_archive` | always: the candidates are the archive's own account (`account.js`) |
//! | others | never (proposed only) |
//!
//! Adopted handles are stored in `meta.me_adopted` (a JSON array of
//! `kind:value` strings, in the same transaction as the import), used by
//! resolution exactly like `[me] identities` and reported in the import
//! response (`self_adopted`). Setting `[me] identities` in config replaces
//! them entirely.

use openstance_import::{HandleKind, ImportStats};
use rusqlite::{Connection, OptionalExtension};

use super::normalize::{normalize_handle, NormalizeContext};
use crate::config::Config;
use crate::error::Result;
use crate::{IdentityId, SourceId};

/// Minimum share of the mbox's `Sent` mail the candidate must have sent.
pub const SENT_SHARE: f64 = 0.8;

const META_KEY: &str = "me_adopted";

/// Handles adopted as "me" by earlier imports (`meta.me_adopted`).
pub fn adopted(conn: &Connection) -> Result<Vec<String>> {
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [META_KEY], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(match v {
        Some(s) => serde_json::from_str(&s)?,
        None => Vec::new(),
    })
}

/// The self handles resolution uses: `[me] identities` when set, else the
/// adopted ones.
pub fn effective(conn: &Connection, cfg: &Config) -> Result<Vec<String>> {
    if cfg.me.identities.is_empty() {
        adopted(conn)
    } else {
        Ok(cfg.me.identities.clone())
    }
}

/// `kind:value` (or a bare email) -> the identity row, if present.
pub(crate) fn identity_of(
    conn: &Connection,
    cfg: &Config,
    spec: &str,
) -> Result<Option<IdentityId>> {
    let (kind, raw) = match spec.split_once(':') {
        Some((k, v)) if HandleKind::parse(k.trim()).is_some() => (k.trim(), v),
        _ if spec.contains('@') => ("email", spec),
        _ => return Ok(None),
    };
    let Some(hk) = HandleKind::parse(kind) else {
        return Ok(None);
    };
    let ctx = NormalizeContext {
        default_region: Some(cfg.import.default_region.clone()).filter(|r| !r.trim().is_empty()),
        importer_kind: String::new(),
    };
    let Some(value) = normalize_handle(hk, raw, &ctx) else {
        return Ok(None);
    };
    Ok(conn
        .prepare_cached("SELECT id FROM identity WHERE kind = ?1 AND value = ?2")?
        .query_row([kind, value.as_str()], |r| r.get(0))
        .optional()?)
}

/// Adopt this import's unambiguous self candidates when `[me] identities`
/// is empty. Returns the newly adopted handles (already adopted ones are
/// not repeated). Call inside the import transaction, before resolution.
pub fn adopt_from_import(
    conn: &Connection,
    cfg: &Config,
    kind: &str,
    source_id: SourceId,
    stats: &ImportStats,
) -> Result<Vec<String>> {
    if !cfg.me.identities.is_empty() || stats.self_candidates.is_empty() {
        return Ok(Vec::new());
    }
    let picked: Vec<String> = match kind {
        "x_archive" => stats.self_candidates.clone(),
        "linkedin_csv" if stats.self_candidates.len() == 1 => stats.self_candidates.clone(),
        "mbox" if stats.self_candidates.len() == 1 => {
            let c = &stats.self_candidates[0];
            if sent_share(conn, cfg, source_id, c)? >= SENT_SHARE {
                vec![c.clone()]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    };
    let mut all = adopted(conn)?;
    let mut new = Vec::new();
    for c in picked {
        // Kept even when not (yet) in the graph, like `[me] identities`: the
        // X archive names the owner's handle but only emits account ids.
        if !all.contains(&c) {
            all.push(c.clone());
            new.push(c);
        }
    }
    if !new.is_empty() {
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [META_KEY, &serde_json::to_string(&all)?],
        )?;
    }
    Ok(new)
}

/// Share of this source's `Sent`-labelled email sent from `spec`.
fn sent_share(conn: &Connection, cfg: &Config, source_id: SourceId, spec: &str) -> Result<f64> {
    let Some(id) = identity_of(conn, cfg, spec)? else {
        return Ok(0.0);
    };
    let (sent, from_c): (i64, i64) = conn.query_row(
        "SELECT count(*),
                count(*) FILTER (WHERE EXISTS (
                  SELECT 1 FROM interaction_person ip
                  WHERE ip.interaction_id = i.id AND ip.identity_id = ?2 AND ip.role = 'from'))
         FROM interaction i
         WHERE i.source_id = ?1 AND i.kind = 'email' AND i.dir_hint = 'out'",
        rusqlite::params![source_id, id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if sent == 0 {
        0.0
    } else {
        from_c as f64 / sent as f64
    })
}
