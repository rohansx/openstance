//! Search: parse -> structured filter (SQL) -> FTS5 candidates -> fusion ->
//! optional rerank -> evidence. One function serves the CLI, MCP and HTTP.
//! Spec: `docs/tech-spec/search.md` (P1-CORE-14..17, P1-MODEL-02).
//!
//! Owned by the "search" stream. Module map:
//! - [`rules`]: `RuleParser` (the default `QueryParser`) and its lexicon;
//! - [`filters`]: structured filters to SQL with bound parameters only;
//! - [`fts`]: `person_fts` maintenance, `escape_query`, bm25 candidates;
//! - [`fuse`]: min-max normalization and the score lift;
//! - [`evidence`]: recent interactions, evidence lines, matched fields;
//! - [`person`]: the `Person` / `PersonDetail` JSON builders (`who`).

use openstance_model::{Models, Query, QueryPatch};
use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::json::SearchResponse;

pub mod evidence;
pub mod filters;
pub mod fts;
pub mod fuse;
pub mod person;
pub mod rules;

/// Inputs to [`search`] and their provenance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchRequest {
    /// Raw user text, parsed in step 1 unless `no_parse`.
    pub text: Option<String>,
    /// Explicit flags / JSON filters; always win over parsed values.
    pub filters: QueryPatch,
    /// Skip step 1: text goes straight to `Query.text`.
    pub no_parse: bool,
    /// Include timings (`--explain`).
    pub explain: bool,
    /// Unix seconds at local midnight, for relative dates.
    pub today: i64,
    /// Unix seconds, for evidence (`at <= now`).
    pub now: i64,
}

/// Run a search. `models.parser` is used for step 1 unless the config says
/// `rules` (then core's `RuleParser` runs, with a lexicon from the graph).
pub fn search(
    conn: &Connection,
    cfg: &Config,
    models: &Models,
    req: &SearchRequest,
) -> Result<SearchResponse> {
    let _ = (conn, cfg, models, req);
    todo!("search stream: P1-CORE-16 search")
}

/// The CLI command that reproduces `q` with no model call
/// (`openstance search --company anthropic --lost-touch`).
pub fn rerun_command(q: &Query) -> String {
    let _ = q;
    todo!("search stream: rerun_command")
}
