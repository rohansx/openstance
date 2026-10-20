//! Bulk-sender detection. Spec: `docs/tech-spec/ingestion.md` section 8 and
//! `scoring.md` section 3 (P1-CORE-10).
//!
//! Owned by the "score" stream. A person is bulk (score 0, components 0,
//! hidden from search and ties) when any rule fires: volume without reply
//! (>= `min_messages` from them, no out to them, no replies either way),
//! list headers (>= 3 messages, > ratio with `list_unsub`), a no-reply local
//! part with no `out` interaction, or role-only auto-submitted mail. An
//! `is_bulk` override always wins.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::Connection;

use super::load::{self, Agg};
use super::ScoreConfig;
use crate::error::Result;
use crate::PersonId;

/// `^(no-?reply|do-?not-?reply|notifications?|mailer-daemon|bounces?|newsletter|news|updates|info|marketing|support)([+._-].*)?@`
pub fn is_noreply_address(email: &str) -> bool {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^(no-?reply|do-?not-?reply|notifications?|mailer-daemon|bounces?|newsletter|news|updates|info|marketing|support)([+._-].*)?@",
        )
        .expect("static regex")
    });
    RE.is_match(email)
}

/// Why a person is bulk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BulkRule {
    /// An `is_bulk = '1'` override.
    Override,
    /// >= `min_messages` from them, no reply either way.
    Volume,
    /// > `list_unsubscribe_ratio` of >= 3 messages carried list headers.
    ListHeaders,
    /// A no-reply local part and nothing sent to them.
    NoReply,
    /// Only ever the sender of auto-submitted / list mail.
    AutoSubmitted,
}

impl BulkRule {
    /// Stable name for JSON and `who`.
    pub fn as_str(self) -> &'static str {
        match self {
            BulkRule::Override => "override",
            BulkRule::Volume => "volume_without_reply",
            BulkRule::ListHeaders => "list_headers",
            BulkRule::NoReply => "noreply_address",
            BulkRule::AutoSubmitted => "auto_submitted",
        }
    }
}

/// The rule that makes `person` bulk, or `None` (also for unknown ids).
pub fn classify(
    conn: &Connection,
    cfg: &ScoreConfig,
    person: PersonId,
) -> Result<Option<BulkRule>> {
    let aggs = load::load(conn, Some(&[person]), i64::MAX)?;
    match aggs.get(&person) {
        Some(a) => rule(conn, cfg, a),
        None => Ok(None),
    }
}

/// The first rule that fires for one loaded person. The self person is
/// never bulk; an override always wins.
pub(crate) fn rule(conn: &Connection, cfg: &ScoreConfig, a: &Agg) -> Result<Option<BulkRule>> {
    if a.is_self {
        return Ok(None);
    }
    match a.bulk_override {
        Some(true) => return Ok(Some(BulkRule::Override)),
        Some(false) => return Ok(None),
        None => {}
    }
    // 1. Volume without conversation: nothing addressed to them, no reply
    //    by them (implied: a reply needs an earlier `out`), and the owner
    //    never wrote on any thread they appear in.
    if a.n_from >= cfg.bulk_min_messages
        && a.n_out_any == 0
        && a.inputs.n_out == 0
        && !load::owner_wrote_on_threads(conn, &a.thread_keys)?
    {
        return Ok(Some(BulkRule::Volume));
    }
    // 2. List headers on more than the ratio of at least 3 messages.
    if a.n_from_email >= 3
        && f64::from(a.n_from_unsub) / f64::from(a.n_from_email) > cfg.bulk_list_unsubscribe_ratio
    {
        return Ok(Some(BulkRule::ListHeaders));
    }
    // 3. A no-reply local part, at least one message from them, and nothing
    //    ever sent to them.
    if a.n_out_any == 0 && a.n_from >= 1 && a.emails.iter().any(|e| is_noreply_address(e)) {
        return Ok(Some(BulkRule::NoReply));
    }
    // 4. Only ever the sender of auto-submitted / list mail.
    if a.n_all >= 1 && a.only_auto {
        return Ok(Some(BulkRule::AutoSubmitted));
    }
    Ok(None)
}

/// Set `person.is_bulk` for `people` (or everyone when `None`). Returns the
/// number of bulk people among them.
pub fn detect(conn: &Connection, cfg: &ScoreConfig, people: Option<&[PersonId]>) -> Result<u64> {
    let aggs = load::load(conn, people, i64::MAX)?;
    let mut st = conn.prepare("UPDATE person SET is_bulk = ?1 WHERE id = ?2")?;
    let mut n = 0;
    for (id, a) in &aggs {
        let bulk = rule(conn, cfg, a)?.is_some();
        n += u64::from(bulk);
        st.execute(rusqlite::params![bulk as i64, id])?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noreply_addresses() {
        for a in [
            "noreply@x.example",
            "no-reply@x.example",
            "donotreply@x.example",
            "do-not-reply@x.example",
            "notification@x.example",
            "notifications+abc@x.example",
            "mailer-daemon@x.example",
            "bounce@x.example",
            "bounces-123@x.example",
            "newsletter@x.example",
            "news@digest.example",
            "updates.weekly@x.example",
            "info@x.example",
            "marketing_team@x.example",
            "support@x.example",
            "NoReply@X.example",
        ] {
            assert!(is_noreply_address(a), "{a}");
        }
        for a in [
            "priya@northwind.example",
            "newsome@x.example",
            "information@x.example",
            "supporter@x.example",
            "jane.noreply@x.example",
            "noreply",
            "",
        ] {
            assert!(!is_noreply_address(a), "{a}");
        }
    }
}
