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

/// Set `person.is_bulk` for `people` (or everyone when `None`). Returns the
/// number of bulk people among them.
pub fn detect(conn: &Connection, cfg: &ScoreConfig, people: Option<&[PersonId]>) -> Result<u64> {
    let _ = (conn, cfg, people);
    todo!("score stream: P1-CORE-10 bulk detect")
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
