//! Forward-only, ordered SQL migrations recorded in `meta` and mirrored to
//! `PRAGMA user_version`. Spec: `docs/tech-spec/data-model.md` section 8.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Error, Result};

/// One embedded migration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Schema version after applying it (1-based, contiguous).
    pub version: u32,
    /// File stem, e.g. `0001_init`.
    pub name: &'static str,
    /// The SQL.
    pub sql: &'static str,
}

/// Every migration this binary ships, in order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init",
        sql: include_str!("../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "0002_person_vec",
        sql: include_str!("../../migrations/0002_person_vec.sql"),
    },
];

/// Newest schema version this binary knows.
pub const LATEST_VERSION: u32 = 2;

/// What [`migrate`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateReport {
    /// Version before.
    pub from: u32,
    /// Version after.
    pub to: u32,
    /// Names of the migrations applied, in order.
    pub applied: Vec<String>,
    /// Backup written before migrating a non-empty database.
    pub backup: Option<PathBuf>,
}

/// Current schema version: `meta.schema_version`, or 0 for an empty database.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        return Ok(0);
    }
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    match v {
        None => Ok(0),
        Some(s) => s.trim().parse().map_err(|_| Error::Migration {
            name: "meta".into(),
            message: format!("meta.schema_version is not a number: {s:?}"),
        }),
    }
}

/// Apply every pending migration in [`MIGRATIONS`]. `db_path` (when the
/// database is a file) is where the pre-migration backup goes. `now` is the
/// unix time recorded in `migration.NNNN_name`.
pub fn migrate(conn: &mut Connection, db_path: Option<&Path>, now: i64) -> Result<MigrateReport> {
    migrate_with(conn, db_path, MIGRATIONS, now)
}

/// [`migrate`] over an explicit migration list (tests).
pub fn migrate_with(
    conn: &mut Connection,
    db_path: Option<&Path>,
    migrations: &[Migration],
    now: i64,
) -> Result<MigrateReport> {
    let latest = migrations.last().map_or(0, |m| m.version);
    let from = schema_version(conn)?;
    if from > latest {
        return Err(Error::SchemaTooNew {
            found: from,
            supported: latest,
        });
    }
    let pending: Vec<&Migration> = migrations.iter().filter(|m| m.version > from).collect();
    let mut report = MigrateReport {
        from,
        to: from,
        applied: Vec::new(),
        backup: None,
    };
    if pending.is_empty() {
        return Ok(report);
    }
    if from > 0 {
        if let Some(path) = db_path {
            report.backup = Some(backup(conn, path, from)?);
        }
    }
    for m in pending {
        apply_one(conn, m, now)?;
        report.to = m.version;
        report.applied.push(m.name.to_string());
    }
    Ok(report)
}

fn apply_one(conn: &mut Connection, m: &Migration, now: i64) -> Result<()> {
    let fail = |e: rusqlite::Error| Error::Migration {
        name: m.name.to_string(),
        message: e.to_string(),
    };
    let tx = conn.transaction().map_err(fail)?;
    tx.execute_batch(m.sql).map_err(fail)?;
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [m.version.to_string()],
    )
    .map_err(fail)?;
    tx.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [
            format!("migration.{}", m.name),
            format!("{now}:{}", checksum(m.sql)),
        ],
    )
    .map_err(fail)?;
    tx.pragma_update(None, "user_version", m.version)
        .map_err(fail)?;
    tx.commit().map_err(fail)?;
    Ok(())
}

/// Copy the database to `<path>.bak-v<version>` with SQLite's online backup
/// API (so the WAL is included), keeping the two most recent backups.
fn backup(conn: &Connection, path: &Path, version: u32) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "graph.db".to_string());
    let bak = path.with_file_name(format!("{file_name}.bak-v{version}"));
    conn.backup(rusqlite::DatabaseName::Main, &bak, None)?;
    prune_backups(path, &file_name)?;
    Ok(bak)
}

fn prune_backups(path: &Path, file_name: &str) -> Result<()> {
    let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    let prefix = format!("{file_name}.bak-v");
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| Error::io("listing", dir, e))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let v = name.strip_prefix(&prefix)?.parse::<u32>().ok()?;
            Some((v, e.path()))
        })
        .collect();
    found.sort();
    while found.len() > 2 {
        let (_, p) = found.remove(0);
        std::fs::remove_file(&p).map_err(|e| Error::io("removing old backup", &p, e))?;
    }
    Ok(())
}

/// `'blake3:<hex>'`-free hex digest of a migration's SQL, as recorded in meta.
pub fn checksum(sql: &str) -> String {
    blake3::hash(sql.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open, open_in_memory, DbOptions};

    fn meta(c: &Connection, key: &str) -> Option<String> {
        c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .ok()
    }

    fn user_version(c: &Connection) -> u32 {
        c.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn empty_database_is_version_zero() {
        let c = open_in_memory().unwrap();
        assert_eq!(schema_version(&c).unwrap(), 0);
    }

    #[test]
    fn fresh_database_migrates_to_latest_and_records_history() {
        let mut c = open_in_memory().unwrap();
        let r = migrate(&mut c, None, 1_790_000_000).unwrap();
        assert_eq!(r.from, 0);
        assert_eq!(r.to, LATEST_VERSION);
        assert_eq!(
            r.applied,
            vec!["0001_init".to_string(), "0002_person_vec".to_string()]
        );
        assert_eq!(r.backup, None);
        assert_eq!(schema_version(&c).unwrap(), LATEST_VERSION);
        assert_eq!(user_version(&c), LATEST_VERSION);
        assert_eq!(meta(&c, "schema_version"), Some(LATEST_VERSION.to_string()));
        assert_eq!(meta(&c, "normalizer_version").as_deref(), Some("1"));
        let rec = meta(&c, "migration.0001_init").unwrap();
        assert_eq!(rec, format!("1790000000:{}", checksum(MIGRATIONS[0].sql)));
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut c = open_in_memory().unwrap();
        migrate(&mut c, None, 1).unwrap();
        let r = migrate(&mut c, None, 2).unwrap();
        assert!(r.applied.is_empty());
        assert_eq!((r.from, r.to), (LATEST_VERSION, LATEST_VERSION));
        // history keeps the original applied_at
        assert!(meta(&c, "migration.0001_init").unwrap().starts_with("1:"));
    }

    #[test]
    fn newer_database_is_refused_and_left_untouched() {
        let mut c = open_in_memory().unwrap();
        migrate(&mut c, None, 1).unwrap();
        c.execute(
            "UPDATE meta SET value = '7' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
        c.execute_batch("PRAGMA user_version = 7").unwrap();
        let err = migrate(&mut c, None, 2).unwrap_err();
        match err {
            Error::SchemaTooNew { found, supported } => {
                assert_eq!((found, supported), (7, LATEST_VERSION));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(meta(&c, "schema_version").as_deref(), Some("7"));
        assert_eq!(user_version(&c), 7);
    }

    const EXTRA: Migration = Migration {
        version: 2,
        name: "0002_extra",
        sql: "CREATE TABLE extra (x INTEGER);",
    };

    #[test]
    fn migrating_a_non_empty_file_database_backs_it_up_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.db");
        let mut c = open(&path, &DbOptions::default()).unwrap();
        migrate_with(&mut c, Some(&path), &MIGRATIONS[..1], 1).unwrap();
        c.execute(
            "INSERT INTO source(kind, imported_at) VALUES ('vcf', 1)",
            [],
        )
        .unwrap();

        let r = migrate_with(&mut c, Some(&path), &[MIGRATIONS[0], EXTRA], 2).unwrap();
        assert_eq!(r.applied, vec!["0002_extra".to_string()]);
        let bak = r.backup.expect("backup path");
        assert_eq!(bak, dir.path().join("graph.db.bak-v1"));
        let b = Connection::open(&bak).unwrap();
        let n: i64 = b
            .query_row("SELECT count(*) FROM source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let has_extra: i64 = b
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'extra'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_extra, 0, "backup is the pre-migration state");
        assert_eq!(schema_version(&c).unwrap(), 2);
        assert_eq!(user_version(&c), 2);
    }

    #[test]
    fn failing_migration_rolls_back_and_reports_its_name() {
        let mut c = open_in_memory().unwrap();
        migrate_with(&mut c, None, &MIGRATIONS[..1], 1).unwrap();
        let bad = Migration {
            version: 2,
            name: "0002_bad",
            sql: "CREATE TABLE ok_part (x INTEGER); THIS IS NOT SQL;",
        };
        let err = migrate_with(&mut c, None, &[MIGRATIONS[0], bad], 2).unwrap_err();
        assert!(
            matches!(&err, Error::Migration { name, .. } if name == "0002_bad"),
            "{err:?}"
        );
        assert_eq!(schema_version(&c).unwrap(), 1);
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'ok_part'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    /// P2-CORE-01: a v0.2.0 database (schema 1) gains `person_vec` and
    /// `person_embed` with every existing row intact, and a backup.
    #[test]
    fn v1_database_upgrades_to_vectors_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.db");
        let mut c = open(&path, &DbOptions::default()).unwrap();
        migrate_with(&mut c, Some(&path), &MIGRATIONS[..1], 1).unwrap();
        c.execute_batch(
            "INSERT INTO source(id, kind, imported_at) VALUES (1, 'vcf', 1);
             INSERT INTO person(id, display_name, score) VALUES (10, 'Ana Example', 0.5);
             INSERT INTO identity(id, kind, value, source_id, person_id)
               VALUES (10, 'email', 'a@example.com', 1, 10);",
        )
        .unwrap();
        let r = migrate(&mut c, Some(&path), 2).unwrap();
        assert_eq!(r.applied, vec!["0002_person_vec".to_string()]);
        assert!(r.backup.is_some());
        let name: String = c
            .query_row("SELECT display_name FROM person WHERE id = 10", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "Ana Example");
        assert_eq!(meta(&c, "person_vec_dims").as_deref(), Some("384"));
        let blob = crate::db::vec::to_blob(&[0.5f32; 384]);
        c.execute(
            "INSERT INTO person_vec(person_id, embedding) VALUES (10, ?1)",
            [&blob],
        )
        .unwrap();
        let d: f64 = c
            .query_row(
                "SELECT vec_distance_cosine(embedding, ?1) FROM person_vec WHERE person_id = 10",
                [&blob],
                |r| r.get(0),
            )
            .unwrap();
        assert!(d.abs() < 1e-6);
    }

    #[test]
    fn shipped_migrations_are_contiguous() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
            assert!(m.name.starts_with(&format!("{:04}_", i + 1)));
        }
        assert_eq!(MIGRATIONS.last().unwrap().version, LATEST_VERSION);
    }
}
