//! openstance core: schema, migrations, resolution, scoring and search over
//! one local SQLite graph. Synchronous; depends on the trait crates only.
//!
//! The [`Graph`] facade is the API every surface (CLI, MCP, HTTP) uses. It
//! owns the connection, the config and the providers, opens one `IMMEDIATE`
//! transaction per write, and delegates to the modules below:
//!
//! | module | owner (Phase 1 stream) | role |
//! |---|---|---|
//! | [`db`], [`config`], [`json`], [`stats`], [`clock`] | foundation | connection, migrations, config, JSON shapes, counts |
//! | [`ingest`], [`resolve`], [`overrides`], [`rebuild`] | resolve | Sink, normalization, rules, review, merge/split, rebuild |
//! | [`score`] | score | components, presets, bulk, ties |
//! | [`search`], [`export`] | search | RuleParser, filters, FTS, evidence, export |
//!
//! Cross-module calls are one-directional: `rebuild` calls `resolve`,
//! `score` and `search::fts`; `ingest` calls only `resolve::normalize`;
//! `score` and `search` never call `resolve` or `rebuild`.
#![warn(missing_docs)]

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, TransactionBehavior};

pub mod clock;
pub mod config;
pub mod db;
pub mod egress;
pub mod embed;
pub mod error;
pub mod export;
pub mod graphq;
pub mod ingest;
pub mod json;
pub mod keys;
pub mod overrides;
pub mod playbook;
pub mod rebuild;
pub mod resolve;
pub mod score;
pub mod search;
pub mod stats;
pub mod view;

pub use clock::{Clock, FixedClock, SystemClock};
pub use config::{Config, Paths};
pub use error::{Error, Result};
pub use openstance_enrich as enrich;
pub use openstance_import as import;
pub use openstance_model as model;
pub use openstance_model::{Query, QueryPatch};

use crate::export::{ExportFormat, ExportOptions, ExportReport};
use crate::ingest::{ImportRequest, StageOutcome, Touched};
use crate::json::{ImportResponse, PersonDetail, ReviewQueue, SearchResponse, SourceImport};
use crate::overrides::Edit;
use crate::rebuild::RebuildReport;
use crate::resolve::commands::{MergeOutcome, SplitOutcome};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::person::DetailOptions;
use crate::search::SearchRequest;
use crate::stats::GraphCounts;

/// A person id: the anchor identity id of the person's cluster.
pub type PersonId = i64;
/// An `identity.id`.
pub type IdentityId = i64;
/// A `source.id`.
pub type SourceId = i64;

/// The concrete implementations the binary wires in from config.
pub struct Providers {
    /// Importers compiled into the binary, in detection order.
    pub importers: Vec<Box<dyn openstance_import::Importer>>,
    /// Embedder, parser, reranker.
    pub models: openstance_model::Models,
    /// Enrichment backend (Phase 3; `NullEnricher` until then).
    pub enricher: Box<dyn openstance_enrich::Enricher>,
}

impl Providers {
    /// No importers, null models, null enricher.
    pub fn null() -> Self {
        Providers {
            importers: Vec::new(),
            models: openstance_model::Models::null(),
            enricher: Box::new(openstance_enrich::NullEnricher),
        }
    }

    /// Null providers with the given importers.
    pub fn with_importers(importers: Vec<Box<dyn openstance_import::Importer>>) -> Self {
        Providers {
            importers,
            ..Providers::null()
        }
    }
}

impl std::fmt::Debug for Providers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Providers")
            .field(
                "importers",
                &self.importers.iter().map(|i| i.kind()).collect::<Vec<_>>(),
            )
            .field("models", &self.models)
            .field("enricher", &self.enricher.name())
            .finish()
    }
}

/// Options for [`Graph::init`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitOptions {
    /// `--me` handles (`email:...` or a bare email), written to `[me] identities`.
    pub me: Vec<String>,
}

/// What [`Graph::init`] found or created.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InitReport {
    /// `$OPENSTANCE_HOME`.
    pub home: PathBuf,
    /// Database file.
    pub db: PathBuf,
    /// Config file.
    pub config_file: PathBuf,
    /// The home directory was created.
    pub created_home: bool,
    /// The database was created.
    pub created_db: bool,
    /// The config file was written.
    pub created_config: bool,
    /// Schema version after init.
    pub schema_version: u32,
}

/// One local relationship graph.
pub struct Graph {
    conn: Connection,
    cfg: Config,
    providers: Providers,
    clock: Box<dyn Clock>,
    embed: embed::Hook,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph")
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

/// Run `f` inside one `BEGIN IMMEDIATE` transaction: commit on `Ok`, roll
/// back on `Err` (and on panic, when the transaction is dropped).
pub fn write_tx<T>(conn: &mut Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(busy_or_db)?;
    let out = f(&tx)?;
    tx.commit().map_err(busy_or_db)?;
    Ok(out)
}

fn busy_or_db(e: rusqlite::Error) -> Error {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            Error::Busy
        }
        _ => Error::Db(e),
    }
}

impl Graph {
    /// Create `paths.home` (0700), the database (migrated) and a commented
    /// `config.toml` when missing. Idempotent: an existing setup is left
    /// unchanged and reported.
    pub fn init(cfg: &Config, paths: &Paths, opts: &InitOptions, now: i64) -> Result<InitReport> {
        let created_home = !paths.home.exists();
        if created_home {
            std::fs::create_dir_all(&paths.home)
                .map_err(|e| Error::io("creating", &paths.home, e))?;
            set_private(&paths.home, 0o700)?;
        }
        let created_db = !paths.db.exists();
        let mut conn = db::open(
            &paths.db,
            &db::DbOptions {
                busy_timeout_ms: cfg.db.busy_timeout_ms,
            },
        )?;
        let report = db::migrate(&mut conn, Some(&paths.db), now)?;
        let created_config = !paths.config_file.exists();
        if created_config {
            let me: Vec<String> = opts.me.iter().map(|h| normalize_me_handle(h)).collect();
            if let Some(dir) = paths
                .config_file
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
            {
                std::fs::create_dir_all(dir).map_err(|e| Error::io("creating", dir, e))?;
            }
            std::fs::write(&paths.config_file, config::init_file_contents(&me))
                .map_err(|e| Error::io("writing", &paths.config_file, e))?;
            set_private(&paths.config_file, 0o600)?;
        }
        Ok(InitReport {
            home: paths.home.clone(),
            db: paths.db.clone(),
            config_file: paths.config_file.clone(),
            created_home,
            created_db,
            created_config,
            schema_version: report.to,
        })
    }

    /// Open (and migrate) the database at `db_path`.
    pub fn open(
        cfg: Config,
        db_path: &Path,
        providers: Providers,
        clock: Box<dyn Clock>,
    ) -> Result<Graph> {
        let mut conn = db::open(
            db_path,
            &db::DbOptions {
                busy_timeout_ms: cfg.db.busy_timeout_ms,
            },
        )?;
        db::migrate(&mut conn, Some(db_path), clock.now())?;
        Ok(Graph {
            conn,
            cfg,
            providers,
            clock,
            embed: embed::Hook::default(),
        })
    }

    /// A migrated in-memory graph (tests, `--dry-run`).
    pub fn open_in_memory(
        cfg: Config,
        providers: Providers,
        clock: Box<dyn Clock>,
    ) -> Result<Graph> {
        let mut conn = db::open_in_memory()?;
        db::migrate(&mut conn, None, clock.now())?;
        Ok(Graph {
            conn,
            cfg,
            providers,
            clock,
            embed: embed::Hook::default(),
        })
    }

    /// The connection, for read-only helpers and tests.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The effective config.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The providers.
    pub fn providers(&self) -> &Providers {
        &self.providers
    }

    /// Current time from the injected clock.
    pub fn now(&self) -> i64 {
        self.clock.now()
    }

    /// Run `f` in one `IMMEDIATE` write transaction.
    pub fn write<T>(&mut self, f: impl FnOnce(&Connection, &Config) -> Result<T>) -> Result<T> {
        let cfg = &self.cfg;
        write_tx(&mut self.conn, |tx| f(tx, cfg))
    }

    /// Row counts and resolution totals.
    pub fn counts(&self) -> Result<GraphCounts> {
        stats::counts(&self.conn)
    }

    /// Import files, each in its own transaction, in the order given:
    /// detect (or `--as`), stage raw rows, run rebuild-lite, store stats,
    /// commit, checkpoint the WAL. A failure stops the run; earlier files
    /// stay committed.
    pub fn import(&mut self, paths: &[PathBuf], req: &ImportRequest) -> Result<ImportResponse> {
        let started = std::time::Instant::now();
        let Graph {
            conn,
            cfg,
            providers,
            clock,
            ..
        } = self;
        let importers = &providers.importers;
        let before = stats::summary(conn)?.people;
        let mut sources = Vec::with_capacity(paths.len());
        for path in paths {
            let importer = match &req.kind {
                Some(kind) => openstance_import::by_kind(importers, kind).ok_or_else(|| {
                    let supported: Vec<&str> = importers.iter().map(|i| i.kind()).collect();
                    Error::BadRequest(format!(
                        "unknown importer kind {kind:?}; supported kinds: {}",
                        supported.join(", ")
                    ))
                })?,
                None => openstance_import::detect(importers, path)?,
            };
            let now = clock.now();
            let entry = write_tx(conn, |tx| {
                let outcome = ingest::stage_file(tx, importer, path, req, cfg, now)?;
                if let StageOutcome::Staged(st) = &outcome {
                    if !req.no_resolve {
                        rebuild::incremental(tx, cfg, &st.touched, now)?;
                    }
                    ingest::finish_source(tx, st.source_id, &st.stats)?;
                }
                Ok(source_import(path, importer.kind(), &outcome))
            })?;
            checkpoint(conn)?;
            sources.push(entry);
        }
        let mut resolution = stats::summary(conn)?;
        resolution.people_created = resolution.people.saturating_sub(before);
        if !req.no_resolve {
            self.refresh_embeddings(None)?;
        }
        Ok(ImportResponse {
            v: json::V,
            sources,
            resolution,
            took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// Full rebuild of all derived data.
    pub fn rebuild(&mut self) -> Result<RebuildReport> {
        let now = self.now();
        let report = self.write(|tx, cfg| rebuild::rebuild(tx, cfg, now))?;
        checkpoint(&self.conn)?;
        self.refresh_embeddings(None)?;
        Ok(report)
    }

    /// The review queue.
    pub fn review(&self) -> Result<ReviewQueue> {
        resolve::review::queue(&self.conn, &self.cfg)
    }

    /// Confirm a flagged merge, then re-resolve.
    pub fn review_confirm(&mut self, person: &PersonKey) -> Result<Vec<i64>> {
        let now = self.now();
        let out = self.write(|tx, cfg| {
            let p = resolve_person(tx, person)?.person;
            let ids = resolve::review::confirm(tx, p, now)?;
            rebuild::incremental(tx, cfg, &touched_people(&[p]), now)?;
            Ok(ids)
        })?;
        self.refresh_embeddings(None)?;
        Ok(out)
    }

    /// Reject a suggestion (`split_from` between the two), then re-resolve.
    pub fn review_reject(&mut self, a: &PersonKey, b: &PersonKey) -> Result<i64> {
        let now = self.now();
        let out = self.write(|tx, cfg| {
            let (a, b) = (resolve_person(tx, a)?.person, resolve_person(tx, b)?.person);
            let id = resolve::review::reject(tx, a, b, now)?;
            rebuild::incremental(tx, cfg, &touched_people(&[a, b]), now)?;
            Ok(id)
        })?;
        self.refresh_embeddings(None)?;
        Ok(out)
    }

    /// Apply field edits (overrides) to a person and refresh its derived
    /// fields, score and FTS row. Returns the person id.
    pub fn edit(&mut self, person: &PersonKey, edit: &Edit) -> Result<PersonId> {
        let now = self.now();
        let out = self.write(|tx, cfg| {
            let p = resolve_person(tx, person)?.person;
            overrides::apply_edit(tx, p, edit, now)?;
            rebuild::refresh_people(tx, cfg, &[p], now)?;
            Ok(p)
        })?;
        self.refresh_embeddings(Some(&[out]))?;
        Ok(out)
    }

    /// Merge people (writes `merged_into`), then re-resolve.
    pub fn merge(&mut self, people: &[PersonKey]) -> Result<MergeOutcome> {
        let now = self.now();
        let out = self.write(|tx, cfg| {
            let ids = people
                .iter()
                .map(|k| resolve_person(tx, k).map(|r| r.person))
                .collect::<Result<Vec<_>>>()?;
            let out = resolve::commands::merge(tx, &ids, now)?;
            rebuild::incremental(tx, cfg, &touched_people(&ids), now)?;
            Ok(out)
        })?;
        self.refresh_embeddings(None)?;
        Ok(out)
    }

    /// Split identities out of a person (pins both sides), then re-resolve.
    pub fn split(&mut self, person: &PersonKey, identities: &[IdentityId]) -> Result<SplitOutcome> {
        let now = self.now();
        let out = self.write(|tx, cfg| {
            let p = resolve_person(tx, person)?.person;
            let out = resolve::commands::split(tx, p, identities, now)?;
            rebuild::incremental(tx, cfg, &touched_people(&[out.kept, out.moved]), now)?;
            Ok(out)
        })?;
        self.refresh_embeddings(None)?;
        Ok(out)
    }

    /// Search.
    pub fn search(&self, req: &SearchRequest) -> Result<SearchResponse> {
        search::search(&self.conn, &self.cfg, &self.providers.models, req)
    }

    /// Person detail (`who`) with the default limits.
    pub fn who(&self, person: &PersonKey) -> Result<PersonDetail> {
        self.get_person(person, DetailOptions::default())
    }

    /// `get_person`: the full `PersonDetail` of a person reference
    /// (P2-CORE-04). Limits are clamped to 50.
    pub fn get_person(&self, person: &PersonKey, opts: DetailOptions) -> Result<PersonDetail> {
        graphq::get_person(&self.conn, &self.cfg, person, opts, self.now())
    }

    /// `how_connected`: warm paths to a target in the graph (P2-CORE-05).
    pub fn how_connected(
        &self,
        target: &PersonKey,
        opts: graphq::HowOptions,
    ) -> Result<graphq::HowConnectedResponse> {
        graphq::how_connected(&self.conn, target, opts, self.now())
    }

    /// Export people.
    pub fn export(
        &self,
        format: ExportFormat,
        opts: &ExportOptions,
        out: &mut dyn Write,
    ) -> Result<ExportReport> {
        export::export(&self.conn, &self.cfg, format, opts, out, self.now())
    }
}

fn touched_people(ids: &[PersonId]) -> Touched {
    Touched {
        identities: Default::default(),
        people: ids.iter().copied().collect(),
    }
}

fn checkpoint(conn: &Connection) -> Result<()> {
    let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(())
}

fn source_import(path: &Path, kind: &str, outcome: &StageOutcome) -> SourceImport {
    match outcome {
        StageOutcome::AlreadyImported(src) => SourceImport {
            id: src.id,
            kind: src.kind.clone(),
            label: src.label.clone(),
            path: path.display().to_string(),
            identities: 0,
            interactions: 0,
            skipped_duplicate_file: true,
            stats: Default::default(),
        },
        StageOutcome::Staged(st) => SourceImport {
            id: st.source_id,
            kind: kind.to_string(),
            label: Some(st.label.clone()),
            path: path.display().to_string(),
            identities: st.stats.identities_seen,
            interactions: st.stats.interactions_new,
            skipped_duplicate_file: false,
            stats: st.stats.clone(),
        },
    }
}

/// `--me jane@example.com` is shorthand for `email:jane@example.com`.
fn normalize_me_handle(h: &str) -> String {
    let h = h.trim();
    match h.split_once(':') {
        Some((kind, _)) if openstance_import::HandleKind::parse(kind).is_some() => h.to_string(),
        _ if h.contains('@') => format!("email:{h}"),
        _ => h.to_string(),
    }
}

#[cfg(unix)]
fn set_private(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Error::io("setting permissions on", path, e))
}

#[cfg(not(unix))]
fn set_private(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> Paths {
        Paths {
            home: dir.join("home"),
            config_file: dir.join("home").join("config.toml"),
            db: dir.join("home").join("graph.db"),
        }
    }

    #[test]
    fn init_creates_home_db_and_commented_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let r = Graph::init(
            &Config::default(),
            &p,
            &InitOptions {
                me: vec!["owner@example.com".into()],
            },
            100,
        )
        .unwrap();
        assert!(r.created_home && r.created_db && r.created_config);
        assert_eq!(r.schema_version, db::LATEST_VERSION);
        assert!(p.db.exists());
        let text = std::fs::read_to_string(&p.config_file).unwrap();
        let (cfg, warnings) = config::parse_str(&text, true).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            cfg.me.identities,
            vec!["email:owner@example.com".to_string()]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&p.home), 0o700);
            assert_eq!(mode(&p.config_file), 0o600);
            assert_eq!(mode(&p.db), 0o600);
        }
    }

    #[test]
    fn init_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        Graph::init(&Config::default(), &p, &InitOptions::default(), 1).unwrap();
        std::fs::write(&p.config_file, "[search]\ndefault_limit = 7\n").unwrap();
        let r = Graph::init(
            &Config::default(),
            &p,
            &InitOptions {
                me: vec!["email:x@example.com".into()],
            },
            2,
        )
        .unwrap();
        assert!(!r.created_home && !r.created_db && !r.created_config);
        assert_eq!(
            std::fs::read_to_string(&p.config_file).unwrap(),
            "[search]\ndefault_limit = 7\n"
        );
    }

    #[test]
    fn open_migrates_and_exposes_counts() {
        let dir = tempfile::tempdir().unwrap();
        let g = Graph::open(
            Config::default(),
            &dir.path().join("g.db"),
            Providers::null(),
            Box::new(FixedClock(5)),
        )
        .unwrap();
        assert_eq!(db::schema_version(g.conn()).unwrap(), db::LATEST_VERSION);
        assert_eq!(g.counts().unwrap(), GraphCounts::default());
        assert_eq!(g.now(), 5);
    }

    #[test]
    fn write_commits_on_ok_and_rolls_back_on_err() {
        let mut g = Graph::open_in_memory(
            Config::default(),
            Providers::null(),
            Box::new(FixedClock(0)),
        )
        .unwrap();
        g.write(|tx, _| {
            tx.execute(
                "INSERT INTO source(kind, imported_at) VALUES ('vcf', 1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let err = g
            .write(|tx, _| -> Result<()> {
                tx.execute(
                    "INSERT INTO source(kind, imported_at) VALUES ('vcf', 2)",
                    [],
                )?;
                Err(Error::Internal("boom".into()))
            })
            .unwrap_err();
        assert!(matches!(err, Error::Internal(_)));
        assert_eq!(g.counts().unwrap().sources, 1);
    }

    #[test]
    fn import_rejects_unknown_kind_listing_supported() {
        let mut g = Graph::open_in_memory(
            Config::default(),
            Providers::null(),
            Box::new(FixedClock(0)),
        )
        .unwrap();
        let err = g
            .import(
                &[PathBuf::from("x.csv")],
                &ImportRequest {
                    kind: Some("nope".into()),
                    ..ImportRequest::default()
                },
            )
            .unwrap_err();
        assert!(matches!(err, Error::BadRequest(m) if m.contains("nope")));
    }

    #[test]
    fn providers_debug_lists_ids() {
        let s = format!("{:?}", Providers::null());
        assert!(s.contains("none"), "{s}");
    }
}
