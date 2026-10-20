//! Connection opening (pragmas) and the migration runner.
//! Spec: `docs/tech-spec/data-model.md` sections 7 and 8.

use std::path::Path;

use rusqlite::Connection;

use crate::error::{Error, Result};

pub mod migrate;

pub use migrate::{migrate, schema_version, MigrateReport, Migration, LATEST_VERSION, MIGRATIONS};

/// Connection settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbOptions {
    /// `PRAGMA busy_timeout` in milliseconds (`[db] busy_timeout_ms`).
    pub busy_timeout_ms: u32,
}

impl Default for DbOptions {
    fn default() -> Self {
        DbOptions {
            busy_timeout_ms: 5000,
        }
    }
}

/// Open (creating if needed) the database at `path` and apply the
/// connection pragmas. Creates the parent directory (mode 0700 when it is
/// created) and gives the database file mode 0600. Does not migrate.
pub fn open(path: &Path, opts: &DbOptions) -> Result<Connection> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io("creating", parent, e))?;
            set_mode(parent, 0o700)?;
        }
    }
    let existed = path.exists();
    let conn = Connection::open(path)?;
    if !existed {
        set_mode(path, 0o600)?;
    }
    apply_pragmas(&conn, opts)?;
    Ok(conn)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Error::io("setting permissions on", path, e))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// An in-memory database with the same pragmas (tests, `--dry-run`).
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_pragmas(&conn, &DbOptions::default())?;
    Ok(conn)
}

/// Apply the per-connection pragmas. Called by [`open`] and nowhere else.
fn apply_pragmas(conn: &Connection, opts: &DbOptions) -> Result<()> {
    // journal_mode returns a row; query it rather than executing it.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.execute_batch(&format!(
        "PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = {};
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 268435456;
         PRAGMA cache_size = -65536;",
        opts.busy_timeout_ms
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pragma_i64(c: &Connection, name: &str) -> i64 {
        c.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn open_sets_documented_pragmas() {
        let dir = tempfile::tempdir().unwrap();
        let c = open(&dir.path().join("graph.db"), &DbOptions::default()).unwrap();
        let mode: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(pragma_i64(&c, "synchronous"), 1, "NORMAL");
        assert_eq!(pragma_i64(&c, "foreign_keys"), 1);
        assert_eq!(pragma_i64(&c, "busy_timeout"), 5000);
        assert_eq!(pragma_i64(&c, "temp_store"), 2, "MEMORY");
        assert_eq!(pragma_i64(&c, "cache_size"), -65536);
    }

    #[test]
    fn open_respects_busy_timeout_option() {
        let dir = tempfile::tempdir().unwrap();
        let c = open(
            &dir.path().join("g.db"),
            &DbOptions {
                busy_timeout_ms: 1234,
            },
        )
        .unwrap();
        assert_eq!(pragma_i64(&c, "busy_timeout"), 1234);
    }

    #[test]
    fn open_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("graph.db");
        open(&path, &DbOptions::default()).unwrap();
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let path = home.join("graph.db");
        open(&path, &DbOptions::default()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
    }

    #[test]
    fn in_memory_has_foreign_keys_on() {
        let c = open_in_memory().unwrap();
        assert_eq!(pragma_i64(&c, "foreign_keys"), 1);
    }
}
