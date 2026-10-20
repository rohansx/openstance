//! `mbox` importer (Phase 1, feature `mbox`): Google Takeout and any
//! mboxo/mboxrd file, headers only. Spec: `docs/tech-spec/ingestion.md` 6.1.
//!
//! Owned by the "import" stream. Behaviour to implement:
//! - stream-split on `From ` lines with a 1 MiB reader, never buffering bodies
//!   (`memchr::memmem` to skip to the next `\nFrom `);
//! - parse header blocks (cap 256 KiB -> `SkipReason::HeaderTooLarge`) with
//!   `mail-parser`: From/To/Cc/Bcc/Date/Subject/Message-ID/In-Reply-To/
//!   References/List-Unsubscribe/List-Id/Precedence/X-Gmail-Labels/X-GM-THRID/
//!   Auto-Submitted, RFC 2047 decoding, address groups;
//! - `at` from `Date`, fallback to the `From ` line date, else `BadDate` skip;
//! - `external_id` = Message-ID (angle brackets stripped, lowercased);
//! - `thread_key` = X-GM-THRID, else first References id, else In-Reply-To,
//!   else own Message-ID;
//! - `dir_hint = Out` when X-Gmail-Labels contains `Sent`;
//! - `list_unsub` from List-Unsubscribe / Precedence / Auto-Submitted;
//! - skip Spam/Trash (unless `include_spam`) and Chat as `Unsupported`;
//! - report the dominant `Sent` From address in `ImportStats.self_candidates`;
//! - `email_bodies = true` returns `ImportError::Unsupported` in Phase 1.

use std::path::Path;

use crate::{Detection, ImportError, ImportOptions, ImportStats, Importer, Sink};

/// Headers-only mbox importer.
#[derive(Debug, Clone)]
pub struct MboxImporter {
    opts: ImportOptions,
}

impl MboxImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        MboxImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for MboxImporter {
    fn kind(&self) -> &'static str {
        "mbox"
    }

    /// Strong: first non-empty line starts with `From ` and the first 64 KiB
    /// contain `Message-ID:` or `Date:`. Weak: `.mbox` extension.
    fn detect(&self, _path: &Path) -> Detection {
        todo!("import stream: P1-IMP-02/05 mbox detect")
    }

    fn import(&self, _path: &Path, _sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        todo!("import stream: P1-IMP-02 mbox import")
    }
}
