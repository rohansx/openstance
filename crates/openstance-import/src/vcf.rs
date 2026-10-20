//! `vcf` importer (Phase 1, feature `vcf`): Apple / Google Contacts vCard
//! 2.1, 3.0 and 4.0. Spec: `docs/tech-spec/ingestion.md` 6.3.
//!
//! Owned by the "import" stream. Behaviour to implement:
//! - line unfolding (CRLF + space/tab), QUOTED-PRINTABLE + CHARSET, grouped
//!   properties (`item1.URL`), `ENCODING=b` photos skipped undecoded;
//! - one card = one `record` (UID, else the card ordinal); every handle is a
//!   `RawIdentity` with that record and the card's hints, `observed_at` = REV;
//! - FN (else N reassembled) -> hints.name; EMAIL -> Email; TEL -> Phone;
//!   ORG first component -> hints.company; TITLE -> hints.title; ADR
//!   locality/region -> hints.location; URL / X-SOCIALPROFILE / IMPP ->
//!   LinkedinUrl / XHandle / IgHandle; NOTE/BDAY/PHOTO/CATEGORIES ignored;
//! - a card with only a name emits `NameLocal("vcf|<name>")`; a card with
//!   neither handle nor name is skipped (`Malformed`).

use std::path::Path;

use crate::{Detection, ImportError, ImportOptions, ImportStats, Importer, Sink};

/// vCard importer.
#[derive(Debug, Clone)]
pub struct VcfImporter {
    opts: ImportOptions,
}

impl VcfImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        VcfImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for VcfImporter {
    fn kind(&self) -> &'static str {
        "vcf"
    }

    /// Strong: starts with `BEGIN:VCARD` (after BOM/whitespace).
    /// Weak: `.vcf` / `.vcard` extension.
    fn detect(&self, _path: &Path) -> Detection {
        todo!("import stream: P1-IMP-04/05 vcf detect")
    }

    fn import(&self, _path: &Path, _sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        todo!("import stream: P1-IMP-04 vcf import")
    }
}
