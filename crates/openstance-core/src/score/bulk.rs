//! Bulk-sender detection. Spec: `docs/tech-spec/ingestion.md` section 8 and
//! `scoring.md` section 3 (P1-CORE-10).
//!
//! Owned by the "score" stream. A person is bulk (score 0, components 0,
//! hidden from search and ties) when any rule fires: volume without reply
//! (>= `min_messages` from them, no out to them, no replies either way),
//! list headers (>= 3 messages, > ratio with `list_unsub`), a no-reply local
//! part with no `out` interaction, or role-only auto-submitted mail. An
//! `is_bulk` override always wins.

use rusqlite::Connection;

use super::ScoreConfig;
use crate::error::Result;
use crate::PersonId;

/// `^(no-?reply|do-?not-?reply|notifications?|mailer-daemon|bounces?|newsletter|news|updates|info|marketing|support)([+._-].*)?@`
pub fn is_noreply_address(email: &str) -> bool {
    let _ = email;
    todo!("score stream: is_noreply_address")
}

/// Set `person.is_bulk` for `people` (or everyone when `None`). Returns the
/// number of bulk people among them.
pub fn detect(conn: &Connection, cfg: &ScoreConfig, people: Option<&[PersonId]>) -> Result<u64> {
    let _ = (conn, cfg, people);
    todo!("score stream: P1-CORE-10 bulk detect")
}
