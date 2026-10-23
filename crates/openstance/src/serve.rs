//! `openstance serve` (interfaces.md section 10, P4-API-01/02/06): the
//! local HTTP API, MCP over HTTP at `/mcp`, and the embedded web UI on
//! `127.0.0.1:7411`.
//!
//! - Binds loopback only. A non-loopback `--host` (or `serve.host`) is
//!   refused unless `--i-understand-this-exposes-my-graph` is given on the
//!   command line; config alone is not enough.
//! - The bearer token lives in `serve.token_path` (0600). `init` creates it,
//!   or the first `serve` does for older installs. `--rotate-token`
//!   replaces it; `--print-token` prints it and exits.
//! - Prints the `http://127.0.0.1:7411/#token=...` link for the UI once.
//!   The token never goes to the log.
//!
//! The tokio runtime exists only inside this command.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use clap::Args;

use crate::commands::{usage, Ctx};

/// `openstance serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Port (default serve.port, 7411; 0 picks a free one).
    #[arg(long)]
    pub port: Option<u16>,
    /// Bind address (default serve.host, 127.0.0.1). Anything but loopback
    /// also needs --i-understand-this-exposes-my-graph.
    #[arg(long)]
    pub host: Option<String>,
    /// Do not serve the web UI.
    #[arg(long)]
    pub no_ui: bool,
    /// Serve only /health and MCP at /mcp (no HTTP API, no UI).
    #[arg(long)]
    pub mcp_only: bool,
    /// Replace the bearer token (the old one stops working).
    #[arg(long)]
    pub rotate_token: bool,
    /// Print the bearer token and exit.
    #[arg(long)]
    pub print_token: bool,
    /// Allow binding a non-loopback address. Anyone who can reach it and has
    /// the token can read and change your graph.
    #[arg(long = "i-understand-this-exposes-my-graph")]
    pub expose: bool,
}

/// The token file: `serve.token_path`, `~/.openstance` meaning the home.
pub(crate) fn token_path(ctx: &Ctx) -> std::path::PathBuf {
    let user_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    openstance_core::config::expand_path(
        &ctx.loaded.config.serve.token_path,
        &ctx.loaded.paths.home,
        user_home.as_deref(),
    )
}

/// The address to bind, refusing non-loopback without the flag.
#[cfg_attr(not(feature = "server"), allow(dead_code))]
fn bind_addr(host: &str, port: u16, expose: bool) -> Result<SocketAddr> {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = if h.eq_ignore_ascii_case("localhost") {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        h.parse()
            .map_err(|_| usage(format!("--host {host:?} is not an IP address")))?
    };
    if !ip.is_loopback() {
        if !expose {
            return Err(usage(format!(
                "refusing to bind {ip}: it is not a loopback address. Anyone who can reach it \
                 could try to read your graph. Pass --i-understand-this-exposes-my-graph on the \
                 command line to do it anyway (serve.host in the config is not enough)"
            )));
        }
        eprintln!(
            "warning: binding {ip}, which is reachable from other machines. The bearer token is \
             the only thing protecting your graph; use HTTPS (a tunnel) and keep the token secret."
        );
    }
    Ok(SocketAddr::new(ip, port))
}

/// Run `serve`.
#[cfg(feature = "server")]
pub fn run(ctx: &Ctx, a: ServeArgs) -> Result<()> {
    use anyhow::Context as _;
    use openstance_api::pool::{ClockFn, GraphPool};
    use openstance_api::token;
    use std::io::Write as _;
    use std::sync::Arc;

    let cfg = ctx.loaded.config.clone();
    let path = token_path(ctx);
    let loaded = if a.rotate_token {
        token::rotate(&path)
    } else {
        token::ensure(&path)
    }
    .with_context(|| format!("api token {}", path.display()))?;
    if a.print_token {
        println!("{}", loaded.token.expose());
        return Ok(());
    }
    if a.rotate_token {
        eprintln!("rotated the api token in {}", path.display());
    }
    let host = a.host.clone().unwrap_or_else(|| cfg.serve.host.clone());
    let addr = bind_addr(&host, a.port.unwrap_or(cfg.serve.port), a.expose)?;

    let providers = ctx.providers(&cfg, crate::commands::importers(&cfg))?;
    let now = ctx.now;
    let fixed = ctx.fixed_now();
    let clock: ClockFn = Arc::new(move || {
        if fixed {
            Box::new(openstance_core::FixedClock(now)) as Box<dyn openstance_core::Clock>
        } else {
            Box::new(openstance_core::SystemClock)
        }
    });
    let db = &ctx.loaded.paths.db;
    let pool = GraphPool::open(cfg.clone(), db, providers, clock, READERS)
        .with_context(|| format!("opening {}", db.display()))?;
    let user = std::env::var("USER").ok();
    let opts = openstance_api::ServeOptions {
        port: addr.port(),
        cors_origins: cfg.serve.cors_origins.clone(),
        max_upload_bytes: usize::try_from(u64::from(cfg.serve.max_upload_mb) * 1024 * 1024)
            .unwrap_or(usize::MAX),
        ui: cfg.serve.ui && !a.no_ui && !a.mcp_only,
        api: !a.mcp_only,
        mcp: true,
        token: loaded.token.clone(),
        share: Some(crate::view::dirs(ctx)),
        owner_label: Some(openstance_core::view::owner_label(&cfg, user.as_deref())),
        config_doc: crate::commands::config_json(&ctx.loaded),
        ..openstance_api::ServeOptions::default()
    };
    let ui = opts.ui;
    let api = opts.api;
    let json = ctx.json;
    let token_file = path.display().to_string();
    let secret = loaded.token.expose().to_string();
    tracing::info!(addr = %addr, ui, api, "starting openstance serve");
    openstance_api::run_blocking(addr, pool, opts, move |local| {
        let shown = if local.ip().is_unspecified() {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), local.port())
        } else {
            local
        };
        let base = format!("http://{shown}");
        let ui_link = ui.then(|| format!("{base}/#token={secret}"));
        let mcp = format!("{base}/mcp");
        let mut out = std::io::stdout().lock();
        if json {
            let v = serde_json::json!({
                "v": openstance_core::json::V,
                "api": api.then(|| base.clone()),
                "ui": ui_link,
                "mcp": mcp,
                "token_file": token_file,
            });
            let _ = writeln!(out, "{v}");
        } else {
            if api {
                let _ = writeln!(
                    out,
                    "openstance api  {base}   (bearer token in {token_file})"
                );
            }
            if let Some(l) = &ui_link {
                let _ = writeln!(out, "web ui          {l}   (open this link once)");
            }
            let _ = writeln!(out, "mcp             {mcp}");
            if !openstance_api::ui::embedded() && ui {
                let _ = writeln!(
                    out,
                    "note: this binary has no web UI built in; run `npm run build` in ui/ and rebuild with --features ui"
                );
            }
        }
        let _ = out.flush();
    })
}

/// Reader connections next to the one writer.
#[cfg(feature = "server")]
const READERS: usize = 4;

/// Without the `server` feature there is nothing to serve.
#[cfg(not(feature = "server"))]
pub fn run(_ctx: &Ctx, _a: ServeArgs) -> Result<()> {
    Err(usage(
        "this build has no HTTP server (rebuild with the `server` feature)",
    ))
}

/// Create the api token if missing (`openstance init`, P4-API-02).
#[cfg(feature = "server")]
pub(crate) fn ensure_token(ctx: &Ctx) -> Result<bool> {
    let path = token_path(ctx);
    Ok(openstance_api::token::ensure(&path)?.created)
}

/// Without the server there is no token to create.
#[cfg(not(feature = "server"))]
pub(crate) fn ensure_token(_ctx: &Ctx) -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_binds_need_no_flag() {
        assert_eq!(
            bind_addr("127.0.0.1", 7411, false).unwrap(),
            "127.0.0.1:7411".parse().unwrap()
        );
        assert_eq!(
            bind_addr("localhost", 1, false).unwrap(),
            "127.0.0.1:1".parse().unwrap()
        );
        assert_eq!(
            bind_addr("::1", 2, false).unwrap(),
            "[::1]:2".parse().unwrap()
        );
        assert!(bind_addr("[::1]", 2, false).is_ok());
    }

    #[test]
    fn other_binds_need_the_flag() {
        for h in ["0.0.0.0", "192.168.1.10", "::"] {
            let e = bind_addr(h, 7411, false).unwrap_err();
            assert!(e.to_string().contains("--i-understand"), "{e}");
            assert!(bind_addr(h, 7411, true).is_ok());
        }
        assert!(bind_addr("example.com", 7411, true).is_err());
    }
}
