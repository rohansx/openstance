//! `openstance mcp` (interfaces.md section 10): the MCP server on stdio.
//!
//! stdout is the protocol and nothing else; logs go to stderr (the global
//! `tracing` subscriber already writes there). The tokio runtime exists only
//! inside this command.

use std::path::Path;

use anyhow::Result;
use clap::Args;
use openstance_core::view::ShareDirs;
use openstance_core::{Clock, Config, Providers};

/// `openstance mcp`.
#[derive(Debug, Args)]
#[command(
    long_about = "Run the MCP server on stdio (stdin/stdout carry JSON-RPC; logs go to stderr).\n\n\
                  Tools: search_people, get_person, how_connected, list_views, search_view\n\
                  (local, free), quote_enrich (prices only), path_to and brief (may spend,\n\
                  only when a call sets allow_egress=true; enrich.backend picks the backend).\n\
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
/// `providers` are the CLI's (the configured embedder, loaded lazily, and
/// the `enrich.backend` enricher), so `search_people` ranks exactly like
/// `openstance search --json` and `path_to` / `brief` price and spend
/// exactly like `openstance path` / `brief`. `share` locates my age key
/// (read, never created here) and the imported views.
pub fn run(
    mut cfg: Config,
    db: &Path,
    providers: Providers,
    clock: Box<dyn Clock>,
    share: ShareDirs,
    a: McpArgs,
) -> Result<()> {
    if a.read_only {
        cfg.mcp.read_only = true;
    }
    serve(cfg, db, providers, clock, share)
}

#[cfg(feature = "server")]
fn serve(
    cfg: Config,
    db: &Path,
    providers: Providers,
    clock: Box<dyn Clock>,
    share: ShareDirs,
) -> Result<()> {
    use anyhow::Context;
    use openstance_core::view::{owner_label, ViewOwner};
    use openstance_core::{keys, Graph};
    let user = std::env::var("USER").ok();
    let owner = ViewOwner {
        // The key is created by `init` or the first `view` / `keys`
        // command; the server only reads it.
        pubkey: keys::load(&share.key).ok().map(|k| k.public()),
        label: Some(owner_label(&cfg, user.as_deref())),
    };
    let g = Graph::open(cfg, db, providers, clock)
        .with_context(|| format!("opening {}", db.display()))?;
    if let Some(w) = g.embedding_warning() {
        tracing::warn!("{w}");
    }
    tracing::info!(
        db = %db.display(),
        enricher = g.providers().enricher.name(),
        "starting mcp server on stdio"
    );
    let views = openstance_mcp::ViewsContext {
        dir: share.views,
        owner,
    };
    openstance_mcp::run_stdio(g, Some(views))
}

#[cfg(not(feature = "server"))]
fn serve(
    _cfg: Config,
    _db: &Path,
    _providers: Providers,
    _clock: Box<dyn Clock>,
    _share: ShareDirs,
) -> Result<()> {
    Err(anyhow::Error::new(crate::UsageError(
        "this build has no MCP server (rebuild with the `server` feature)".into(),
    )))
}
