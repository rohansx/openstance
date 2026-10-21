//! The core error type. Variants describe what the caller can do, and
//! messages never contain contact data (ids and paths only).

use std::path::PathBuf;

use crate::json::ErrorCode;

/// Result alias for core.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite failure.
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    /// The database is locked by another writer.
    #[error("the database is busy (another openstance process is writing); retry")]
    Busy,
    /// File-system failure, with the path the user supplied.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being attempted ("creating", "reading", ...).
        action: &'static str,
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The database was written by a newer openstance.
    #[error("graph.db was written by a newer openstance (schema {found} > {supported}); upgrade the binary")]
    SchemaTooNew {
        /// Version found in the database.
        found: u32,
        /// Newest version this binary knows.
        supported: u32,
    },
    /// A migration failed (the database is unchanged for that migration).
    #[error("migration {name} failed: {message}")]
    Migration {
        /// Migration name, e.g. `0001_init`.
        name: String,
        /// Why.
        message: String,
    },
    /// Invalid configuration.
    #[error("config error at {key}: {message}")]
    Config {
        /// Dotted key path (or file path for parse errors).
        key: String,
        /// Why.
        message: String,
    },
    /// Invalid input or flags.
    #[error("{0}")]
    BadRequest(String),
    /// No such person, source, view...
    #[error("not found: {0}")]
    NotFound(String),
    /// A person reference matched several people.
    #[error("{reference:?} matches {} people", candidates.len())]
    Ambiguous {
        /// The reference as given (an id, email or name the user typed).
        reference: String,
        /// `(person id, display name)` of each candidate.
        candidates: Vec<(i64, String)>,
    },
    /// No importer could be chosen for a file.
    #[error(transparent)]
    Detect(#[from] openstance_import::DetectError),
    /// The importer failed; the import was rolled back.
    #[error("import failed: {0}")]
    Import(#[from] openstance_import::ImportError),
    /// JSON (de)serialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A view is past `expires_at`, or a `.ost` file is not addressed to
    /// my key (both exit 7, sharing-and-views.md section 4.4).
    #[error("{0}")]
    ViewExpired(String),
    /// A bug. The message is safe to show.
    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Shorthand for [`Error::Io`].
    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            action,
            path: path.into(),
            source,
        }
    }

    /// The stable JSON error code (interfaces.md section 1.11).
    pub fn code(&self) -> ErrorCode {
        match self {
            Error::Busy => ErrorCode::DbBusy,
            Error::Db(rusqlite::Error::SqliteFailure(e, _))
                if matches!(
                    e.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                ErrorCode::DbBusy
            }
            Error::SchemaTooNew { .. } => ErrorCode::SchemaTooNew,
            Error::Config { .. } | Error::BadRequest(_) | Error::Detect(_) => ErrorCode::BadRequest,
            Error::NotFound(_) => ErrorCode::NotFound,
            Error::Ambiguous { .. } => ErrorCode::Ambiguous,
            Error::ViewExpired(_) => ErrorCode::ViewExpired,
            _ => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_map_to_documented_exit_codes() {
        assert_eq!(
            Error::SchemaTooNew {
                found: 4,
                supported: 1
            }
            .code()
            .exit_code(),
            10
        );
        assert_eq!(Error::Busy.code().exit_code(), 10);
        assert_eq!(Error::NotFound("person 9".into()).code().exit_code(), 5);
        assert_eq!(
            Error::Ambiguous {
                reference: "Sam".into(),
                candidates: vec![]
            }
            .code()
            .exit_code(),
            6
        );
        assert_eq!(Error::BadRequest("x".into()).code().exit_code(), 2);
        assert_eq!(Error::Internal("x".into()).code().exit_code(), 1);
    }

    #[test]
    fn schema_too_new_message_is_the_documented_one() {
        let e = Error::SchemaTooNew {
            found: 4,
            supported: 3,
        };
        assert_eq!(
            e.to_string(),
            "graph.db was written by a newer openstance (schema 4 > 3); upgrade the binary"
        );
    }
}
