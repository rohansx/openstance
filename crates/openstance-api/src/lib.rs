//! Local HTTP API for openstance (Phase 4). Routes in
//! `docs/tech-spec/interfaces.md` section 3 map onto `openstance-core` calls
//! and return the same JSON as the CLI's `--json` and the MCP tools.
//!
//! - [`token`]: the bearer token file (create, load, rotate, verify).
//! - [`pool`]: one writer and N reader connections on `spawn_blocking`.
//! - [`server`]: the router and its security layers (Host check, Origin /
//!   CORS allow-list, bearer token, body limits, security headers).
//! - [`routes`]: the handlers; [`mcp_http`]: MCP at `/mcp`; [`ui`]: the
//!   embedded web UI.
//!
//! Tokio lives in this crate (and the binary) only; core stays synchronous.
#![warn(missing_docs)]

use std::net::SocketAddr;

/// The core crate this API wraps.
pub use openstance_core as core;
/// The MCP server mounted at `/mcp`.
pub use openstance_mcp as mcp;

pub mod error;
pub mod mcp_http;
pub mod pool;
pub mod routes;
pub mod server;
pub mod token;
pub mod ui;

pub use server::{app, ServeOptions};

/// Serve `router` on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

/// Bind `addr`, call `ready` with the bound address (the port may have
/// been 0), then serve until Ctrl-C, on a new multi-thread runtime. Used by
/// `openstance serve`; every other command stays runtime-free.
pub fn run_blocking(
    addr: SocketAddr,
    pool: pool::GraphPool,
    mut opts: ServeOptions,
    ready: impl FnOnce(SocketAddr),
) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("openstance-serve")
        .build()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let local = listener.local_addr()?;
        opts.port = local.port();
        let router = app(pool, opts);
        ready(local);
        serve(listener, router, async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await?;
        Ok(())
    })
}
