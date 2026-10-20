//! The `openstance` CLI (also installed as `ost`).
//!
//! Every command is a thin layer over the `openstance_core::Graph` facade
//! (`docs/tech-spec/interfaces.md` sections 4 to 6). Output goes to stdout,
//! logs and prompts to stderr; with `--json`, stdout is exactly one JSON
//! document, errors included (`{"v": 1, "error": {...}}`). Exit codes follow
//! interfaces.md section 14.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

mod commands;
mod render;

/// Global flags shared by every command (interfaces.md section 4).
#[derive(Debug, Parser)]
#[command(
    name = "openstance",
    version,
    about = "A local-first relationship graph for your own contacts"
)]
pub struct Cli {
    /// Use another database file.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
    /// Config file.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Print the shared JSON shape instead of a table.
    #[arg(long, global = true)]
    pub json: bool,
    /// Fixed "now" for scoring and relative dates: unix seconds,
    /// YYYY-MM-DD or RFC 3339 (tests, reproducible reports).
    #[arg(long, global = true, value_name = "TIME")]
    pub now: Option<String>,
    /// More logging (repeatable).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// Less logging.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Phase 1 commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create ~/.openstance, graph.db and a commented config.toml.
    Init(InitArgs),
    /// Import export files (mbox, LinkedIn CSV, vcf).
    Import(ImportArgs),
    /// Recompute everything derived from sources and edits.
    Rebuild,
    /// Confirm or split flagged merges.
    Review(ReviewArgs),
    /// Search people.
    Search(SearchArgs),
    /// Show one person: fields, score breakdown, identities, evidence.
    Who(WhoArgs),
    /// Edit a person (writes overrides).
    Edit(EditArgs),
    /// Merge two people.
    Merge(MergeArgs),
    /// Split identities out of a person.
    Split(SplitArgs),
    /// Export people as csv or json.
    Export(ExportArgs),
    /// Print the effective configuration.
    Config(ConfigArgs),
}

/// `openstance init`.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Your own email address (repeatable); written to `[me] identities`.
    #[arg(long, value_name = "EMAIL")]
    pub me: Vec<String>,
}

/// `openstance import`.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Files to import, in order.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Skip detection: mbox, linkedin_csv, vcf.
    #[arg(long, visible_alias = "as", value_name = "IMPORTER")]
    pub kind: Option<String>,
    /// `source.label` (default derived from the file name and date).
    #[arg(long)]
    pub label: Option<String>,
    /// mbox: also parse bodies (not supported in this release).
    #[arg(long)]
    pub bodies: bool,
    /// Replace that source's raw rows with this file.
    #[arg(long, value_name = "SOURCE_ID")]
    pub replace: Option<i64>,
    /// Skip resolution and scoring (run `openstance rebuild` later).
    #[arg(long)]
    pub no_resolve: bool,
}

/// `openstance review`.
#[derive(Debug, Args)]
pub struct ReviewArgs {
    /// List flagged merges and suggestions, then exit.
    #[arg(long)]
    pub list: bool,
    /// Only rows at or above this confidence.
    #[arg(long, value_name = "0..1")]
    pub min_confidence: Option<f64>,
    /// Accept every flagged merge (filtered by --rule / --min-confidence).
    #[arg(long, conflicts_with_all = ["confirm", "split", "reject"])]
    pub confirm_all: bool,
    /// With --confirm-all: only this rule (name_company, name_thread).
    #[arg(long)]
    pub rule: Option<String>,
    /// Same as --confirm-all --min-confidence N.
    #[arg(long, value_name = "N")]
    pub accept_all_above: Option<f64>,
    /// Accept one flagged merge (writes merged_into).
    #[arg(long, value_name = "PERSON")]
    pub confirm: Option<String>,
    /// Split the flagged identities off a person (writes split_from).
    #[arg(long, value_name = "PERSON")]
    pub split: Option<String>,
    /// Reject a suggestion: keep these two people apart.
    #[arg(long, num_args = 2, value_names = ["A", "B"])]
    pub reject: Vec<String>,
}

/// Filters shared by `search` and `export` (the `Query` fields).
#[derive(Debug, Args, Default, Clone)]
pub struct FilterArgs {
    /// Company (repeatable, OR-ed).
    #[arg(long)]
    pub company: Vec<String>,
    /// Title (repeatable, OR-ed).
    #[arg(long)]
    pub title: Vec<String>,
    /// Location (repeatable, OR-ed).
    #[arg(long)]
    pub location: Vec<String>,
    /// Industry (soft: boosts text relevance, never excludes).
    #[arg(long)]
    pub industry: Vec<String>,
    /// Tag (repeatable).
    #[arg(long)]
    pub tag: Vec<String>,
    /// Had an interaction on or after (YYYY, YYYY-MM, YYYY-MM-DD, 90d, 6m, 1y).
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,
    /// Had an interaction before (same forms as --since).
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,
    /// Last contact before (same forms as --since).
    #[arg(long, value_name = "DATE")]
    pub before: Option<String>,
    /// Met in person (has a meeting).
    #[arg(long, conflicts_with = "not_met")]
    pub met: bool,
    /// Never met in person.
    #[arg(long)]
    pub not_met: bool,
    /// They replied (reciprocity > 0).
    #[arg(long, conflicts_with = "not_replied")]
    pub replied: bool,
    /// They never replied.
    #[arg(long)]
    pub not_replied: bool,
    /// Minimum relationship score (0..1).
    #[arg(long, value_name = "N")]
    pub min_score: Option<f32>,
    /// High frequency, low recency.
    #[arg(long, conflicts_with = "not_lost_touch")]
    pub lost_touch: bool,
    /// Exclude lost-touch people.
    #[arg(long, hide = true)]
    pub not_lost_touch: bool,
    /// Include bulk senders (score 0).
    #[arg(long)]
    pub include_bulk: bool,
}

/// `openstance search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Free text, parsed into filters unless --no-parse.
    pub text: Option<String>,
    /// Structured filters (always win over parsed values).
    #[command(flatten)]
    pub filters: FilterArgs,
    /// Maximum results (default 30, max 200).
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// Print the parsed query and the per-result relevance breakdown.
    #[arg(long)]
    pub explain: bool,
    /// Skip the parser; text goes straight to full-text search.
    #[arg(long)]
    pub no_parse: bool,
    /// A full Query as JSON (or `-` for stdin); implies --no-parse.
    #[arg(long, value_name = "JSON|-")]
    pub query_json: Option<String>,
    /// Rules parser, no embedder, no reranker, regardless of config.
    #[arg(long)]
    pub no_model: bool,
}

/// `openstance who`.
#[derive(Debug, Args)]
pub struct WhoArgs {
    /// An id, email, LinkedIn URL or slug (in/x), @handle, or a name.
    pub person: String,
    /// Recent interactions to show.
    #[arg(long, default_value_t = 10)]
    pub interactions: usize,
}

/// `openstance edit`.
#[derive(Debug, Args)]
pub struct EditArgs {
    /// An id, email, LinkedIn URL or slug, @handle, or a unique name.
    pub person: String,
    /// Display name.
    #[arg(long)]
    pub name: Option<String>,
    /// Title.
    #[arg(long)]
    pub title: Option<String>,
    /// Company.
    #[arg(long)]
    pub company: Option<String>,
    /// Location.
    #[arg(long)]
    pub location: Option<String>,
    /// Bio.
    #[arg(long)]
    pub bio: Option<String>,
    /// Restore the derived value of a field (writes a tombstone).
    #[arg(long, value_name = "FIELD")]
    pub unset: Vec<String>,
    /// Add a tag (repeatable).
    #[arg(long)]
    pub tag: Vec<String>,
    /// Remove a tag (repeatable).
    #[arg(long)]
    pub untag: Vec<String>,
    /// Add a note.
    #[arg(long)]
    pub note: Vec<String>,
    /// Never share this person in views.
    #[arg(long, conflicts_with = "share")]
    pub no_share: bool,
    /// Allow sharing again.
    #[arg(long)]
    pub share: bool,
    /// Never send this person to enrichment.
    #[arg(long, conflicts_with = "enrich")]
    pub no_enrich: bool,
    /// Allow enrichment again.
    #[arg(long)]
    pub enrich: bool,
    /// Not a bulk sender (fixes a false positive).
    #[arg(long, conflicts_with = "bulk")]
    pub not_bulk: bool,
    /// Mark as a bulk sender.
    #[arg(long)]
    pub bulk: bool,
    /// Print the person's override rows.
    #[arg(long)]
    pub history: bool,
}

/// `openstance merge`.
#[derive(Debug, Args)]
pub struct MergeArgs {
    /// First person (kept: the smaller anchor id survives).
    pub a: String,
    /// Second person.
    pub b: String,
    /// Remove a forced merge between the two anchor ids.
    #[arg(long)]
    pub undo: bool,
}

/// `openstance split`.
#[derive(Debug, Args)]
pub struct SplitArgs {
    /// The person to split (or the first of two with --undo).
    pub person: String,
    /// With --undo: the other person.
    pub other: Option<String>,
    /// Identity to move to a new person: `kind:value` or an identity id
    /// (repeatable; interactive without it).
    #[arg(long)]
    pub identity: Vec<String>,
    /// Remove the split_from rows between `person` and `other`.
    #[arg(long, requires = "other")]
    pub undo: bool,
}

/// Export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// One row per person; formula cells neutralized.
    Csv,
    /// `{"v": 1, "people": [PersonDetail]}`.
    Json,
}

/// `openstance export`.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Output format.
    #[arg(long, value_enum)]
    pub format: Format,
    /// Write to this file instead of stdout.
    #[arg(short = 'o', long = "out", value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// Only people matching this search text (parsed like `search`).
    #[arg(long, value_name = "TEXT")]
    pub query: Option<String>,
    /// Structured filters.
    #[command(flatten)]
    pub filters: FilterArgs,
    /// Include people marked do-not-share.
    #[arg(long)]
    pub include_do_not_share: bool,
}

/// `openstance config`.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Sub-action; without one, print the effective config.
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

/// `openstance config <action>`.
#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print the config file, database and home paths.
    Path,
    /// Print one key, e.g. `score.half_life_days`.
    Get {
        /// Dotted key.
        key: String,
    },
}

/// Shared entry point for the `openstance` and `ost` binaries.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    init_logging(&cli);
    let json = cli.json;
    match commands::dispatch(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(report_error(&e, json)),
    }
}

fn init_logging(cli: &Cli) {
    use tracing_subscriber::EnvFilter;
    let level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter =
        EnvFilter::try_from_env("OPENSTANCE_LOG").unwrap_or_else(|_| EnvFilter::new(level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}

/// A usage problem the user can fix (exit 2).
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// Print the error (stderr, and the JSON envelope on stdout with `--json`)
/// and return the exit code.
fn report_error(e: &anyhow::Error, json: bool) -> u8 {
    use openstance_core::json::{ErrorBody, ErrorCode, ErrorResponse, V};
    let core = e
        .chain()
        .find_map(|c| c.downcast_ref::<openstance_core::Error>());
    let usage = e.chain().any(|c| c.downcast_ref::<UsageError>().is_some());
    let resp = match core {
        Some(c) => ErrorResponse::from(c),
        None => ErrorResponse {
            v: V,
            error: ErrorBody {
                code: if usage {
                    ErrorCode::BadRequest
                } else {
                    ErrorCode::Internal
                },
                message: format!("{e:#}"),
                candidates: Vec::new(),
            },
        },
    };
    eprintln!("error: {e:#}");
    for c in &resp.error.candidates {
        eprintln!("  {:>6}  {}", c.id, c.display_name);
    }
    if json {
        if let Ok(s) = serde_json::to_string_pretty(&resp) {
            println!("{s}");
        }
    }
    resp.error.code.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn now_accepts_unix_dates_and_rfc3339() {
        assert_eq!(commands::parse_now("1792497600").unwrap(), 1_792_497_600);
        assert_eq!(commands::parse_now("2026-10-20").unwrap(), 1_792_454_400);
        assert_eq!(
            commands::parse_now("2026-10-20T12:00:00Z").unwrap(),
            1_792_497_600
        );
        assert!(commands::parse_now("yesterday-ish").is_err());
    }
}
