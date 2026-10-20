//! `linkedin_csv` importer (Phase 1, feature `linkedin`): `Connections.csv`,
//! `messages.csv`, `Invitations.csv`. Spec: `docs/tech-spec/ingestion.md` 6.2.
//!
//! Owned by the "import" stream. Behaviour to implement:
//! - CSV quirks ported from v0.1: UTF-8 BOM, quoted commas, doubled quotes,
//!   CRLF, multiline values, a `Notes:` preamble of up to 10 lines before the
//!   header, header aliases (`Email Address`/`E-mail Address`,
//!   `Position`/`Title`);
//! - Connections: one `Connection` interaction per row, `at` = Connected On
//!   (00:00 UTC), `dir_hint = Mutual`, external id `conn:<slug>`, one
//!   participant (role `Subject`) with `LinkedinUrl(URL)` and
//!   `Email(Email Address)` if non-empty, hints name/title/company;
//! - messages: one `Dm` per row, `thread_key` = CONVERSATION ID, external id
//!   `msg:<conv>:<date>:<sender slug>`, sender (role `Sender`) and each
//!   recipient (role `Recipient`); CONTENT is never read; spam rows skipped;
//!   opaque member URLs joined to a Connections slug only for unique names;
//! - Invitations: `Connection` kind, `subject = "invitation"`, `dir_hint` from
//!   Direction (INCOMING -> In, OUTGOING -> Out), external id
//!   `inv:<direction><slug>:<sent at>`; Message is never read;
//! - `default_label` = `linkedin <export date>` (zip name or file mtime).

use std::path::Path;

use crate::{Detection, ImportError, ImportOptions, ImportStats, Importer, Sink};

/// Importer for the three LinkedIn export CSVs.
#[derive(Debug, Clone)]
pub struct LinkedinCsvImporter {
    opts: ImportOptions,
}

impl LinkedinCsvImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        LinkedinCsvImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for LinkedinCsvImporter {
    fn kind(&self) -> &'static str {
        "linkedin_csv"
    }

    /// Strong: header row (after an optional `Notes:` preamble) contains
    /// `First Name,Last Name` and `URL`, or `CONVERSATION ID` and
    /// `SENDER PROFILE URL`, or `Direction` and `inviterProfileUrl`.
    /// Weak: file name `Connections.csv`.
    fn detect(&self, _path: &Path) -> Detection {
        todo!("import stream: P1-IMP-03/05 linkedin detect")
    }

    fn import(&self, _path: &Path, _sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        todo!("import stream: P1-IMP-03 linkedin import")
    }
}
