//! `openstance export --format csv|json`: people, identities, score
//! components and applied overrides. `do_not_share` and bulk people are
//! excluded unless asked. CSV cells starting with `= + - @` get a leading
//! apostrophe (formula-injection guard ported from v0.1).
//! Spec: `docs/tech-spec/data-model.md` section 9 (P1-CORE-18).
//!
//! Owned by the "search" stream.

use std::borrow::Cow;
use std::io::Write;

use openstance_model::Query;
use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::error::Result;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One row per person.
    Csv,
    /// `{"v": 1, "people": [PersonDetail-like objects]}`.
    Json,
}

/// What to export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportOptions {
    /// Only people matching this query (`--query` + filters).
    pub query: Option<Query>,
    /// `--include-bulk`.
    pub include_bulk: bool,
    /// `--include-do-not-share`.
    pub include_do_not_share: bool,
}

/// Counts written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExportReport {
    /// People written.
    pub people: u64,
    /// People left out because of `do_not_share`.
    pub excluded_do_not_share: u64,
}

/// Write the export.
pub fn export(
    conn: &Connection,
    cfg: &Config,
    format: ExportFormat,
    opts: &ExportOptions,
    out: &mut dyn Write,
    now: i64,
) -> Result<ExportReport> {
    let _ = (conn, cfg, format, opts, out, now);
    todo!("search stream: P1-CORE-18 export")
}

/// Prefix `'` to cells starting with `=`, `+`, `-` or `@`.
pub fn escape_csv_cell(cell: &str) -> Cow<'_, str> {
    let _ = cell;
    todo!("search stream: P1-CORE-18 escape_csv_cell")
}
