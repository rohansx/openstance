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

use openstance_import::{
    HandleKind, ImportStats, Importer, InteractionKind, RawIdentity, RawInteraction, Role, Sink,
    SkipReason,
};
use rusqlite::{Connection, OptionalExtension};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::resolve::normalize::{normalize_handle, NormalizeContext};
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
    let mut f = std::fs::File::open(path).map_err(|e| Error::io("reading", path, e))?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h).map_err(|e| Error::io("reading", path, e))?;
    Ok(format!("blake3:{}", h.finalize().to_hex()))
}

/// The source with this file hash, if any.
pub fn find_source_by_hash(conn: &Connection, hash: &str) -> Result<Option<SourceRow>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, kind, label, path, imported_at, file_hash FROM source WHERE file_hash = ?1",
        )?
        .query_row([hash], |r| {
            Ok(SourceRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                label: r.get(2)?,
                path: r.get(3)?,
                imported_at: r.get(4)?,
                file_hash: r.get(5)?,
            })
        })
        .optional()?)
}

/// `<kind> <yyyy-mm-dd>` (UTC) for `now`.
fn default_label(kind: &str, now: i64) -> String {
    let day = chrono::DateTime::from_timestamp(now, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    format!("{kind} {day}")
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
    if req.replace.is_some() {
        return Err(Error::BadRequest(
            "--replace is not supported yet; remove the source and import again".into(),
        ));
    }
    let started = std::time::Instant::now();
    let hash = file_hash(path)?;
    if let Some(existing) = find_source_by_hash(conn, &hash)? {
        return Ok(StageOutcome::AlreadyImported(existing));
    }
    let label = req
        .label
        .clone()
        .or_else(|| importer.default_label(path))
        .unwrap_or_else(|| default_label(importer.kind(), now));
    conn.prepare_cached(
        "INSERT INTO source(kind, label, path, imported_at, file_hash, importer_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?
    .execute(rusqlite::params![
        importer.kind(),
        label,
        path.display().to_string(),
        now,
        hash,
        importer.version(),
    ])?;
    let source_id = conn.last_insert_rowid();
    let region = cfg.import.default_region.trim();
    let opts = SinkOptions {
        default_region: (!region.is_empty()).then(|| region.to_string()),
        importer_kind: importer.kind().to_string(),
    };
    let mut sink = GraphSink::new(conn, source_id, opts, now);
    let imported = importer.import(path, &mut sink);
    let report = sink.finish()?;
    let mut stats = imported?;
    stats.interactions_new = report.interactions_new;
    stats.interactions_dup += report.interactions_dup;
    stats.identities_new = report.identities_new;
    stats.identities_seen = report.identities_seen;
    for (reason, n) in &report.skipped {
        let e = stats.skipped.entry(reason.clone()).or_insert(0);
        *e = (*e).max(*n);
    }
    if stats.bytes == 0 {
        stats.bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    stats.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(StageOutcome::Staged(Staged {
        source_id,
        label,
        stats,
        touched: report.touched,
    }))
}

/// Store the final stats on the source row (`source.stats_json`).
pub fn finish_source(conn: &Connection, source_id: SourceId, stats: &ImportStats) -> Result<()> {
    let json = serde_json::to_string(stats)?;
    conn.prepare_cached("UPDATE source SET stats_json = ?1 WHERE id = ?2")?
        .execute(rusqlite::params![json, source_id])?;
    Ok(())
}

/// Options the Sink needs for normalization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkOptions {
    /// `[import] default_region` for phones without `+`.
    pub default_region: Option<String>,
    /// The importer's kind: the `name_local` scope (`"vcf|jane doe"`).
    pub importer_kind: String,
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
    /// Skips by reason key: the larger of what the importer reported
    /// through [`Sink::skip`] and what the Sink dropped itself.
    pub skipped: BTreeMap<String, u64>,
    /// What changed.
    pub touched: Touched,
}

/// Size of the `(kind, value) -> identity.id` cache.
const IDENTITY_CACHE: usize = 100_000;

/// A contact record being assembled from `RawIdentity`s sharing a key.
struct PendingContact {
    record: String,
    observed_at: Option<i64>,
    handles: Vec<(openstance_import::Handle, openstance_import::Hints)>,
}

/// One normalized participant row.
struct Row {
    kind: HandleKind,
    value: String,
    role: Role,
    slot: i64,
    hints: openstance_import::Hints,
}

/// The `Sink` that writes raw rows for one source. `Sink` methods cannot
/// return errors, so the first database error is kept and returned by
/// [`GraphSink::finish`] (which makes the caller roll back).
pub struct GraphSink<'c> {
    conn: &'c Connection,
    source_id: SourceId,
    ctx: NormalizeContext,
    now: i64,
    report: SinkReport,
    first_error: Option<Error>,
    cache: lru::LruCache<(HandleKind, String), IdentityId>,
    contacts: Vec<PendingContact>,
    contact_index: BTreeMap<String, usize>,
    /// Skips reported by the importer through `skip()`.
    received: BTreeMap<String, u64>,
    /// Skips the Sink generated itself (invalid handles, no participants).
    internal: BTreeMap<String, u64>,
}

impl<'c> GraphSink<'c> {
    /// A sink writing rows for `source_id` (which the caller inserted).
    pub fn new(conn: &'c Connection, source_id: SourceId, opts: SinkOptions, now: i64) -> Self {
        GraphSink {
            conn,
            source_id,
            ctx: NormalizeContext {
                default_region: opts.default_region,
                importer_kind: opts.importer_kind,
            },
            now,
            report: SinkReport::default(),
            first_error: None,
            cache: lru::LruCache::new(
                std::num::NonZeroUsize::new(IDENTITY_CACHE).unwrap_or(std::num::NonZeroUsize::MIN),
            ),
            contacts: Vec::new(),
            contact_index: BTreeMap::new(),
            received: BTreeMap::new(),
            internal: BTreeMap::new(),
        }
    }

    /// Flush pending `contact` records and return the counters, or the first
    /// error seen.
    pub fn finish(mut self) -> Result<SinkReport> {
        let contacts = std::mem::take(&mut self.contacts);
        for c in contacts {
            if self.first_error.is_some() {
                break;
            }
            let r = self.write_contact(c);
            self.keep(r);
        }
        if let Some(e) = self.first_error {
            return Err(e);
        }
        let mut skipped = self.received;
        for (k, n) in self.internal {
            *skipped.entry(k).or_insert(0) += n;
        }
        self.report.skipped = skipped;
        Ok(self.report)
    }

    fn keep(&mut self, r: Result<()>) {
        if let Err(e) = r {
            if self.first_error.is_none() {
                self.first_error = Some(e);
            }
        }
    }

    fn internal_skip(&mut self, reason: SkipReason) {
        *self
            .internal
            .entry(reason.as_str().to_string())
            .or_insert(0) += 1;
    }

    fn normalize(&mut self, h: &openstance_import::Handle) -> Option<String> {
        let v = normalize_handle(h.kind, &h.raw, &self.ctx);
        if v.is_none() {
            self.internal_skip(SkipReason::InvalidHandle);
        }
        v
    }

    fn write_contact(&mut self, c: PendingContact) -> Result<()> {
        let mut rows = Vec::with_capacity(c.handles.len());
        for (h, hints) in &c.handles {
            if let Some(value) = self.normalize(h) {
                rows.push(Row {
                    kind: h.kind,
                    value,
                    role: Role::Subject,
                    slot: 0,
                    hints: hints.clone(),
                });
            }
        }
        if rows.is_empty() {
            self.internal_skip(SkipReason::NoParticipants);
            return Ok(());
        }
        let at = c.observed_at.unwrap_or(self.now);
        let ext = format!("contact:{}", c.record);
        self.write("contact", at, None, &ext, None, None, false, rows)
    }

    fn write_interaction(&mut self, r: RawInteraction) -> Result<()> {
        let mut rows = Vec::new();
        for (slot, p) in r.participants.iter().enumerate() {
            for h in &p.handles {
                if let Some(value) = self.normalize(h) {
                    rows.push(Row {
                        kind: h.kind,
                        value,
                        role: p.role,
                        slot: i64::try_from(slot).unwrap_or(i64::MAX),
                        hints: p.hints.clone(),
                    });
                }
            }
        }
        if rows.is_empty() {
            self.internal_skip(SkipReason::NoParticipants);
            return Ok(());
        }
        let kind = r.kind.as_str();
        let subject = r
            .subject
            .as_deref()
            .map(clean_subject)
            .filter(|s| !s.is_empty());
        let ext = match r
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(e) => e.to_string(),
            None => synthesize_id(kind, r.at, &rows, subject.as_deref()),
        };
        if matches!(r.kind, InteractionKind::Email | InteractionKind::Meeting) {
            let exists: bool = self
                .conn
                .prepare_cached(
                    "SELECT EXISTS(SELECT 1 FROM interaction WHERE kind = ?1 AND external_id = ?2)",
                )?
                .query_row(rusqlite::params![kind, ext], |row| row.get(0))?;
            if exists {
                self.report.interactions_dup += 1;
                return Ok(());
            }
        }
        let thread = r
            .thread_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        self.write(
            kind,
            r.at,
            subject.as_deref(),
            &ext,
            thread,
            r.dir_hint.map(|d| d.as_str()),
            r.list_unsub,
            rows,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        kind: &str,
        at: i64,
        subject: Option<&str>,
        external_id: &str,
        thread_key: Option<&str>,
        dir_hint: Option<&str>,
        list_unsub: bool,
        rows: Vec<Row>,
    ) -> Result<()> {
        let inserted = self
            .conn
            .prepare_cached(
                "INSERT OR IGNORE INTO interaction
                   (source_id, kind, at, subject, external_id, thread_key, dir_hint, list_unsub)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(rusqlite::params![
                self.source_id,
                kind,
                at,
                subject,
                external_id,
                thread_key,
                dir_hint,
                list_unsub,
            ])?;
        if inserted == 0 {
            self.report.interactions_dup += 1;
            return Ok(());
        }
        let interaction_id = self.conn.last_insert_rowid();
        self.report.interactions_new += 1;
        for row in rows {
            let identity_id = self.identity_id(row.kind, &row.value)?;
            self.report.identities_seen += 1;
            self.report.touched.identities.insert(identity_id);
            self.conn
                .prepare_cached(
                    "INSERT OR IGNORE INTO interaction_person
                       (interaction_id, identity_id, role, slot,
                        name_hint, title_hint, company_hint, location_hint)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )?
                .execute(rusqlite::params![
                    interaction_id,
                    identity_id,
                    row.role.as_str(),
                    row.slot,
                    clean_hint(row.hints.name.as_deref()),
                    clean_hint(row.hints.title.as_deref()),
                    clean_hint(row.hints.company.as_deref()),
                    clean_hint(row.hints.location.as_deref()),
                ])?;
        }
        Ok(())
    }

    fn identity_id(&mut self, kind: HandleKind, value: &str) -> Result<IdentityId> {
        let key = (kind, value.to_string());
        if let Some(id) = self.cache.get(&key) {
            return Ok(*id);
        }
        let new: Option<i64> = self
            .conn
            .prepare_cached(
                "INSERT INTO identity(kind, value, source_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(kind, value) DO NOTHING RETURNING id",
            )?
            .query_row(
                rusqlite::params![kind.as_str(), value, self.source_id],
                |r| r.get(0),
            )
            .optional()?;
        let id = match new {
            Some(id) => {
                self.report.identities_new += 1;
                id
            }
            None => self
                .conn
                .prepare_cached("SELECT id FROM identity WHERE kind = ?1 AND value = ?2")?
                .query_row(rusqlite::params![kind.as_str(), value], |r| r.get(0))?,
        };
        self.cache.put(key, id);
        Ok(id)
    }
}

/// Strip control characters and truncate to 512 characters.
fn clean_subject(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(openstance_import::MAX_SUBJECT_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn clean_hint(s: Option<&str>) -> Option<String> {
    let t: String = s?
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(1024)
        .collect();
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// `syn:` + first 16 hex of `blake3(kind | at | sorted handles | subject)`.
fn synthesize_id(kind: &str, at: i64, rows: &[Row], subject: Option<&str>) -> String {
    let mut handles: Vec<String> = rows
        .iter()
        .map(|r| format!("{}:{}", r.kind.as_str(), r.value))
        .collect();
    handles.sort();
    handles.dedup();
    let text = format!(
        "{kind}|{at}|{}|{}",
        handles.join(","),
        subject.unwrap_or("")
    );
    let hex = blake3::hash(text.as_bytes()).to_hex();
    format!("syn:{}", &hex.as_str()[..16])
}

impl Sink for GraphSink<'_> {
    fn identity(&mut self, r: RawIdentity) {
        if self.first_error.is_some() {
            return;
        }
        let idx = match self.contact_index.get(&r.record) {
            Some(&i) => i,
            None => {
                self.contacts.push(PendingContact {
                    record: r.record.clone(),
                    observed_at: None,
                    handles: Vec::new(),
                });
                let i = self.contacts.len() - 1;
                self.contact_index.insert(r.record.clone(), i);
                i
            }
        };
        let c = &mut self.contacts[idx];
        if c.observed_at.is_none() {
            c.observed_at = r.observed_at;
        }
        c.handles.push((r.handle, r.hints));
    }

    fn interaction(&mut self, r: RawInteraction) {
        if self.first_error.is_some() {
            return;
        }
        let res = self.write_interaction(r);
        self.keep(res);
    }

    fn skip(&mut self, reason: SkipReason, _locator: &str) {
        *self
            .received
            .entry(reason.as_str().to_string())
            .or_insert(0) += 1;
    }
}
