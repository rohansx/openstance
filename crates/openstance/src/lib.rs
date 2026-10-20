//! The `openstance` CLI (also installed as `ost`).
//!
//! Owned by the "search" stream from Phase 1 stage 2 onward (P1-CLI-01..07).
//! This skeleton only fixes the entry point both binaries share, the global
//! flags and the Phase 1 command list; every command body is still to be
//! written against the `openstance_core::Graph` facade.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Global flags shared by every command (interfaces.md section 4).
#[derive(Debug, Parser)]
#[command(
    name = "openstance",
    version,
    about = "A local-first relationship graph for your own contacts"
)]
pub struct Cli {
    /// Use another database file.
    #[arg(long, global = true, env = "OPENSTANCE_DB")]
    pub db: Option<PathBuf>,
    /// Config file.
    #[arg(long, global = true, env = "OPENSTANCE_CONFIG")]
    pub config: Option<PathBuf>,
    /// Print the shared JSON shape instead of a table.
    #[arg(long, global = true)]
    pub json: bool,
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

/// Phase 1 commands. Arguments are added by the CLI stream.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create ~/.openstance, graph.db and a commented config.toml.
    Init,
    /// Import export files (mbox, LinkedIn CSV, vcf).
    Import,
    /// Recompute everything derived from sources and edits.
    Rebuild,
    /// Confirm or split flagged merges.
    Review,
    /// Search people.
    Search,
    /// Edit a person (writes overrides).
    Edit,
    /// Merge two people.
    Merge,
    /// Split a person.
    Split,
    /// Export people as csv or json.
    Export,
    /// Print the effective configuration.
    Config,
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
    eprintln!("error: `{:?}` is not implemented yet", cli.command);
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
