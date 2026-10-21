//! MCP server for openstance: the tool schemas of
//! `docs/tech-spec/interfaces.md` section 2 mapped onto `openstance-core`.
//!
//! - Tools only (no resources or prompts), the eight of interfaces.md 2.1:
//!   `search_people`, `get_person`, `how_connected`, `list_views` and
//!   `search_view` are local, read-only and free; `quote_enrich` asks the
//!   configured enrichment backend for prices and never spends; `path_to`
//!   and `brief` can spend, but only when the call sets `allow_egress: true`
//!   (otherwise they return `status: "quote"` and send nothing).
//! - Every result carries `structuredContent` (the shared JSON shape, the
//!   same document the CLI prints with `--json`) and a short text block.
//! - Core is synchronous. Calls run on tokio's blocking pool, one at a time
//!   per graph (the [`SharedGraph`] mutex). A panic in a call becomes an
//!   `internal` tool error, not a dead server.
//! - On stdio, stdout carries the protocol and nothing else. Logs go through
//!   `tracing`, which the binary points at stderr.
#![warn(missing_docs)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use openstance_core::Graph;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

pub mod schema;
pub mod text;
pub mod tools;

/// The core crate this server wraps (re-exported for the binary's wiring).
pub use openstance_core as core;
pub use tools::{ToolCtx, ToolOutput};

/// One graph shared by every request of a server.
pub type SharedGraph = Arc<Mutex<Graph>>;

/// Sent to clients in `initialize` (`instructions`).
pub const INSTRUCTIONS: &str = "openstance searches the user's own relationship graph, \
stored on this machine. search_people, get_person, how_connected, list_views and \
search_view are read-only and free, and send nothing anywhere. Use search_people to find \
people (free text plus filters; the parsed filters come back in `query`), get_person for \
everything about one person, and how_connected for warm introduction paths to someone in \
the graph. path_to and brief can look people up outside the graph, which costs money: \
always call them with allow_egress=false first, show the user the quote, and call again \
with allow_egress=true only after the user agrees. quote_enrich only prices requests. \
When you answer, quote the dated evidence lines as given instead of paraphrasing them, \
and never claim a relationship that no evidence line supports.";

/// Where imported views live and who "I" am, for `list_views` and
/// `search_view`. Without it those tools see own views only.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewsContext {
    /// `<home>/views`: imported `.ost` copies.
    pub dir: PathBuf,
    /// The owner shown on own views (my age public key and label).
    pub owner: openstance_core::view::ViewOwner,
}

/// The rmcp server handler.
#[derive(Clone)]
pub struct OpenstanceMcp {
    graph: SharedGraph,
    ctx: Arc<ToolCtx>,
}

impl std::fmt::Debug for OpenstanceMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenstanceMcp").finish_non_exhaustive()
    }
}

impl OpenstanceMcp {
    /// A server over `graph`.
    pub fn new(graph: Graph) -> Self {
        Self::shared(Arc::new(Mutex::new(graph)))
    }

    /// A server over a graph shared with other surfaces.
    pub fn shared(graph: SharedGraph) -> Self {
        OpenstanceMcp {
            graph,
            ctx: Arc::new(ToolCtx::default()),
        }
    }

    /// Serve `list_views` / `search_view` from `views`.
    #[must_use]
    pub fn with_views(mut self, views: ViewsContext) -> Self {
        self.ctx = Arc::new(ToolCtx { views: Some(views) });
        self
    }

    /// Run one tool call on the blocking pool.
    pub async fn run_tool(
        &self,
        name: String,
        args: Option<rmcp::model::JsonObject>,
    ) -> ToolOutput {
        let graph = Arc::clone(&self.graph);
        let ctx = Arc::clone(&self.ctx);
        let tool = name.clone();
        let joined = tokio::task::spawn_blocking(move || {
            // A previous call that panicked poisons the lock; the graph
            // itself is still usable (each call is one read).
            let g = graph
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tools::call_with(&g, &ctx, &tool, args)
        })
        .await;
        match joined {
            Ok(Some(out)) => out,
            Ok(None) => ToolOutput::internal(&format!("no tool named {name}")),
            Err(e) => {
                tracing::error!(tool = %name, error = %e, "tool call panicked");
                ToolOutput::internal(&format!("{name} failed unexpectedly; see the log"))
            }
        }
    }
}

fn to_result(out: ToolOutput) -> CallToolResult {
    let mut r = if out.is_error {
        CallToolResult::structured_error(out.structured)
    } else {
        CallToolResult::structured(out.structured)
    };
    r.content = vec![Content::text(out.text)];
    r
}

impl ServerHandler for OpenstanceMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("openstance", env!("CARGO_PKG_VERSION"))
                    .with_title("openstance"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(schema::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();
        if !schema::is_tool(&name) {
            return Err(McpError::invalid_params(
                format!("unknown tool {name:?}"),
                None,
            ));
        }
        tracing::debug!(tool = %name, "call");
        Ok(to_result(self.run_tool(name, request.arguments).await))
    }
}

/// Serve MCP on this process's stdin/stdout until the client disconnects.
pub async fn serve_stdio(server: OpenstanceMcp) -> anyhow::Result<()> {
    let running = server.serve(rmcp::transport::stdio()).await?;
    tracing::info!("mcp server ready on stdio");
    running.waiting().await?;
    tracing::info!("mcp client disconnected");
    Ok(())
}

/// [`serve_stdio`] on a new multi-thread runtime (the `openstance mcp`
/// command; every other command stays runtime-free).
pub fn run_stdio(graph: Graph, views: Option<ViewsContext>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("openstance-mcp")
        .build()?;
    let mut server = OpenstanceMcp::new(graph);
    if let Some(v) = views {
        server = server.with_views(v);
    }
    rt.block_on(serve_stdio(server))
}
