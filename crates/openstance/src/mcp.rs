//! `openstance mcp` (interfaces.md section 10): the MCP server on stdio.
//!
//! stdout is the protocol and nothing else; logs go to stderr (the global
//! `tracing` subscriber already writes there). The tokio runtime exists only
//! inside this command.

use std::path::Path;

use anyhow::Result;
use clap::Args;
use openstance_core::{Clock, Config, Providers};

/// `openstance mcp`.
#[derive(Debug, Args)]
#[command(
    long_about = "Run the MCP server on stdio (stdin/stdout carry JSON-RPC; logs go to stderr).\n\n\
                  Tools: search_people, get_person, how_connected. All are local and read-only.\n\
                  Claude Code: claude mcp add openstance -- openstance mcp"
)]
pub struct McpArgs {
    /// Force `allow_egress = false` on every tool and refuse edits
    /// (same as `[mcp] read_only = true`).
    #[arg(long)]
    pub read_only: bool,
}

/// Open the graph and serve until the client closes stdin.
///
/// `providers` are the CLI's (the configured embedder, loaded lazily), so
/// `search_people` ranks exactly like `openstance search --json`.
pub fn run(
    mut cfg: Config,
    db: &Path,
    providers: Providers,
    clock: Box<dyn Clock>,
    a: McpArgs,
) -> Result<()> {
    if a.read_only {
        cfg.mcp.read_only = true;
    }
    serve(cfg, db, providers, clock)
}

#[cfg(feature = "server")]
fn serve(cfg: Config, db: &Path, providers: Providers, clock: Box<dyn Clock>) -> Result<()> {
    use anyhow::Context;
    use openstance_core::Graph;
    let g = Graph::open(cfg, db, providers, clock)
        .with_context(|| format!("opening {}", db.display()))?;
    if let Some(w) = g.embedding_warning() {
        tracing::warn!("{w}");
    }
    tracing::info!(db = %db.display(), "starting mcp server on stdio");
    openstance_mcp::run_stdio(g)
}

#[cfg(not(feature = "server"))]
fn serve(_cfg: Config, _db: &Path, _providers: Providers, _clock: Box<dyn Clock>) -> Result<()> {
    Err(anyhow::Error::new(crate::UsageError(
        "this build has no MCP server (rebuild with the `server` feature)".into(),
    )))
}
