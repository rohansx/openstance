//! The staging writer: [`GraphSink`] implements `openstance_import::Sink`
//! and writes the raw tables (`source`, `identity`, `interaction`,
//! `interaction_person`) for one import, inside the caller's transaction.
//! Spec: `docs/tech-spec/ingestion.md` sections 1, 3 and 5 (P1-CORE-05).
//!
//! Owned by the "resolve" stream. This module never calls scoring or search:
//! after [`stage_file`] returns, `Graph::import` hands [`Staged::touched`] to
//! [`crate::rebuild::incremental`], which runs resolution, scoring and FTS.
//!
//! Rules the implementation must follow:
//! - dedupe layer 1: `file_hash = "blake3:" + hex` of the whole file; an
//!   existing hash returns [`StageOutcome::AlreadyImported`] and writes nothing;
//! - dedupe layer 2: `UNIQUE(source_id, external_id)` with `INSERT OR IGNORE`;
//!   synthesized ids are `syn:` + first 16 hex of
//!   `blake3(kind | at | sorted normalized handles | subject)`;
//! - dedupe layer 3: `email`/`meeting` interactions whose `(kind, external_id)`
//!   exists in any source are skipped (`interactions_dup`);
//! - handles are normalized with [`crate::resolve::normalize::normalize_handle`];
//!   invalid ones are dropped (`InvalidHandle`), participants without handles
//!   are dropped, interactions without participants are skipped;
//! - `RawIdentity`s sharing a `record` become one `contact` interaction
//!   (`external_id = "contact:" + record`, one slot, role `subject`);
//! - subjects truncated to 512 chars with control characters stripped;
//! - identity cache: bounded LRU of `(kind, value) -> id`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use openstance_import::{ImportStats, Importer, RawIdentity, RawInteraction, Sink, SkipReason};
use rusqlite::Connection;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::{IdentityId, PersonId, SourceId};

/// Per-import options for `Graph::import`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportRequest {
    /// `--as <kind>`: skip detection.
    pub kind: Option<String>,
    /// `--label`: `source.label` (default derived by the importer or
    /// `<kind> <yyyy-mm-dd>`).
    pub label: Option<String>,
    /// `--replace <source_id>` (delete that source's raw rows first).
    pub replace: Option<SourceId>,
    /// `--no-resolve`: write raw rows only; run `rebuild` later.
    pub no_resolve: bool,
}

/// What the import touched, for incremental re-resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touched {
    /// Identities that gained an observation (new or existing rows).
    pub identities: BTreeSet<IdentityId>,
    /// People whose clusters must be re-derived (merge/split/edit callers).
    pub people: BTreeSet<PersonId>,
}

impl Touched {
    /// True when nothing was touched.
    pub fn is_empty(&self) -> bool {
        self.identities.is_empty() && self.people.is_empty()
    }
}

/// A `source` row as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    /// `source.id`.
    pub id: SourceId,
    /// `source.kind`.
    pub kind: String,
    /// `source.label`.
    pub label: Option<String>,
    /// `source.path`.
    pub path: Option<String>,
    /// `source.imported_at`.
    pub imported_at: i64,
    /// `source.file_hash`.
    pub file_hash: Option<String>,
}

/// Result of a successful staging run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The new `source.id`.
    pub source_id: SourceId,
    /// The label written.
    pub label: String,
    /// Importer stats merged with the Sink's counters.
    pub stats: ImportStats,
    /// What changed.
    pub touched: Touched,
}

/// Outcome of [`stage_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutcome {
    /// The file hash already exists; nothing was written
    /// ("already imported as source #3 ...").
    AlreadyImported(SourceRow),
    /// Raw rows written (not yet committed).
    Staged(Staged),
}

/// `"blake3:<hex>"` of the file, computed streaming.
pub fn file_hash(path: &Path) -> Result<String> {
    let _ = path;
    todo!("resolve stream: P1-CORE-05 file_hash")
}

/// The source with this file hash, if any.
pub fn find_source_by_hash(conn: &Connection, hash: &str) -> Result<Option<SourceRow>> {
    let _ = (conn, hash);
    todo!("resolve stream: P1-CORE-05 find_source_by_hash")
}

/// Run `importer` over `path` and write its raw rows through a
/// [`GraphSink`]. The caller owns the transaction (`BEGIN IMMEDIATE`) and
/// rolls back on `Err`. Does not resolve, score or index.
pub fn stage_file(
    conn: &Connection,
    importer: &dyn Importer,
    path: &Path,
    req: &ImportRequest,
    cfg: &Config,
    now: i64,
) -> Result<StageOutcome> {
    let _ = (conn, importer, path, req, cfg, now);
    todo!("resolve stream: P1-CORE-05 stage_file")
}

/// Store the final stats on the source row (`source.stats_json`).
pub fn finish_source(conn: &Connection, source_id: SourceId, stats: &ImportStats) -> Result<()> {
    let _ = (conn, source_id, stats);
    todo!("resolve stream: P1-CORE-05 finish_source")
}

/// Options the Sink needs for normalization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkOptions {
    /// `[import] default_region` for phones without `+`.
    pub default_region: Option<String>,
}

/// Counters and side information collected by a [`GraphSink`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkReport {
    /// Interactions inserted.
    pub interactions_new: u64,
    /// Interactions skipped as duplicates.
    pub interactions_dup: u64,
    /// Identity rows inserted.
    pub identities_new: u64,
    /// Identity observations.
    pub identities_seen: u64,
    /// Skips by reason key.
    pub skipped: BTreeMap<String, u64>,
    /// What changed.
    pub touched: Touched,
}

/// The `Sink` that writes raw rows for one source. `Sink` methods cannot
/// return errors, so the first database error is kept and returned by
/// [`GraphSink::finish`] (which makes the caller roll back).
pub struct GraphSink<'c> {
    #[allow(dead_code)]
    conn: &'c Connection,
    #[allow(dead_code)]
    source_id: SourceId,
    #[allow(dead_code)]
    opts: SinkOptions,
    #[allow(dead_code)]
    now: i64,
    #[allow(dead_code)]
    report: SinkReport,
    #[allow(dead_code)]
    first_error: Option<Error>,
}

impl<'c> GraphSink<'c> {
    /// A sink writing rows for `source_id` (which the caller inserted).
    pub fn new(conn: &'c Connection, source_id: SourceId, opts: SinkOptions, now: i64) -> Self {
        GraphSink {
            conn,
            source_id,
            opts,
            now,
            report: SinkReport::default(),
            first_error: None,
        }
    }

    /// Flush pending `contact` records and return the counters, or the first
    /// error seen.
    pub fn finish(self) -> Result<SinkReport> {
        todo!("resolve stream: P1-CORE-05 GraphSink::finish")
    }
}

impl Sink for GraphSink<'_> {
    fn identity(&mut self, r: RawIdentity) {
        let _ = r;
        todo!("resolve stream: P1-CORE-05 GraphSink::identity")
    }

    fn interaction(&mut self, r: RawInteraction) {
        let _ = r;
        todo!("resolve stream: P1-CORE-05 GraphSink::interaction")
    }

    fn skip(&mut self, reason: SkipReason, locator: &str) {
        let _ = (reason, locator);
        todo!("resolve stream: P1-CORE-05 GraphSink::skip")
    }
}
