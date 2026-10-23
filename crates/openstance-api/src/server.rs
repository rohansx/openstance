//! The router: security layers around the routes of interfaces.md section 3.
//!
//! Layer order, outermost first: security headers (every response), Host
//! check (421), Origin / CORS (403 for foreign origins, allow headers only
//! for `serve.cors_origins`), body limit (1 MB; uploads have their own),
//! then the routes. The bearer token is checked on every API route and on
//! `/mcp`; `/health` and the static UI are public.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use openstance_core::view::ShareDirs;
use serde_json::Value;

use crate::error::ApiError;
use crate::pool::GraphPool;
use crate::token::Token;
use crate::{mcp_http, routes, ui};

/// Everything the router needs besides the graphs.
#[derive(Clone)]
pub struct ServeOptions {
    /// The port the server listens on (the Host check expects it).
    pub port: u16,
    /// More accepted `Host` values (`ip:port`), for a non-loopback bind the
    /// user explicitly allowed.
    pub extra_hosts: Vec<String>,
    /// `serve.cors_origins`.
    pub cors_origins: Vec<String>,
    /// Request body limit outside uploads (1 MB).
    pub max_body_bytes: usize,
    /// `serve.max_upload_mb` in bytes (`POST /import`).
    pub max_upload_bytes: usize,
    /// Serve the embedded UI (`--no-ui` turns it off).
    pub ui: bool,
    /// Serve the HTTP API (`--mcp-only` turns it off).
    pub api: bool,
    /// Serve MCP at `/mcp`.
    pub mcp: bool,
    /// The bearer token.
    pub token: Token,
    /// My age key and the imported-views directory (views routes).
    pub share: Option<ShareDirs>,
    /// The owner label written into exported views.
    pub owner_label: Option<String>,
    /// The `GET /config` document (`openstance config --json`).
    pub config_doc: Value,
}

impl std::fmt::Debug for ServeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeOptions")
            .field("port", &self.port)
            .field("cors_origins", &self.cors_origins)
            .field("ui", &self.ui)
            .field("api", &self.api)
            .field("mcp", &self.mcp)
            .finish_non_exhaustive()
    }
}

/// Requests handled at once; more wait in line (a browser tab or local
/// script cannot pile up unbounded work).
pub const MAX_IN_FLIGHT: usize = 64;

/// 1 MB.
pub const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            port: 7411,
            extra_hosts: Vec::new(),
            cors_origins: Vec::new(),
            max_body_bytes: DEFAULT_BODY_LIMIT,
            max_upload_bytes: 2048 * 1024 * 1024,
            ui: true,
            api: true,
            mcp: true,
            token: Token::from_secret(String::new()),
            share: None,
            owner_label: None,
            config_doc: serde_json::json!({ "v": 1 }),
        }
    }
}

/// Shared by every handler.
pub struct AppState {
    /// The graphs.
    pub pool: GraphPool,
    /// The options.
    pub opts: ServeOptions,
    hosts: BTreeSet<String>,
    warned_forwarded: AtomicBool,
}

/// Handler state.
pub type St = State<Arc<AppState>>;

/// The full application router.
pub fn app(pool: GraphPool, opts: ServeOptions) -> Router {
    let mut hosts: BTreeSet<String> = ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .map(|h| format!("{h}:{}", opts.port))
        .collect();
    hosts.extend(opts.extra_hosts.iter().map(|h| h.to_ascii_lowercase()));
    let state = Arc::new(AppState {
        pool,
        opts,
        hosts,
        warned_forwarded: AtomicBool::new(false),
    });
    let s = &state.opts;

    let mut protected = Router::new();
    if s.api {
        protected = protected.merge(routes::router(s.max_upload_bytes));
    }
    if s.mcp {
        protected = protected.route_service("/mcp", mcp_http::service(&state));
    }
    let protected = protected.route_layer(middleware::from_fn_with_state(
        Arc::clone(&state),
        require_token,
    ));

    let mut app = Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .method_not_allowed_fallback(|| async { ApiError::method_not_allowed() });
    app = if s.ui && s.api {
        app.fallback(ui::serve)
    } else {
        app.fallback(|| async { ApiError::not_found("no such route") })
    };
    app.layer(middleware::from_fn_with_state(
        Arc::clone(&state),
        limit_body,
    ))
    .layer(middleware::from_fn_with_state(
        Arc::clone(&state),
        origin_policy,
    ))
    .layer(middleware::from_fn_with_state(
        Arc::clone(&state),
        check_host,
    ))
    .layer(middleware::from_fn(security_headers))
    .layer(tower::limit::ConcurrencyLimitLayer::new(MAX_IN_FLIGHT))
    .with_state(state)
}

/// Routes whose body limit is `serve.max_upload_mb` instead of 1 MB.
pub(crate) fn is_upload(path: &str) -> bool {
    matches!(path, "/import" | "/views/import")
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

async fn security_headers(req: Request, next: Next) -> Response {
    let ui_page = ui::is_ui_request(&req);
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    let set = |h: &mut HeaderMap, k: header::HeaderName, v: &'static str| {
        h.insert(k, HeaderValue::from_static(v));
    };
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::CONTENT_SECURITY_POLICY, ui::CSP);
    if !ui_page || !h.contains_key(header::CACHE_CONTROL) {
        set(h, header::CACHE_CONTROL, "no-store");
    }
    resp
}

/// DNS-rebinding defence: only `127.0.0.1:<port>`, `localhost:<port>`,
/// `[::1]:<port>` (plus explicitly allowed bind addresses).
async fn check_host(State(st): St, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            req.uri()
                .authority()
                .map(|a| a.as_str().to_ascii_lowercase())
        });
    match host {
        Some(h) if st.hosts.contains(&h) => {}
        _ => return ApiError::misdirected("unexpected Host header").into_response(),
    }
    if req
        .headers()
        .keys()
        .any(|k| k.as_str().starts_with("x-forwarded-"))
        && !st.warned_forwarded.swap(true, Ordering::Relaxed)
    {
        tracing::warn!(
            "a request carried X-Forwarded-* headers: is `openstance serve` behind a proxy or tunnel? \
             Anyone who can reach the tunnel and has the token can read your graph"
        );
    }
    next.run(req).await
}

/// CORS is off: same-origin requests pass, origins in `serve.cors_origins`
/// get allow headers (and preflights), every other `Origin` is refused.
async fn origin_policy(State(st): St, req: Request, next: Next) -> Response {
    let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok())
        .map(str::to_string)
    else {
        return next.run(req).await;
    };
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if origin.eq_ignore_ascii_case(&format!("http://{host}")) {
        return next.run(req).await;
    }
    if !st.opts.cors_origins.iter().any(|o| o == &origin) {
        return ApiError::forbidden("cross-origin requests are not allowed from this origin")
            .into_response();
    }
    let allow = HeaderValue::from_str(&origin).unwrap_or(HeaderValue::from_static("null"));
    let preflight = req.method() == Method::OPTIONS
        && req
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut resp = if preflight {
        let mut r = StatusCode::NO_CONTENT.into_response();
        let h = r.headers_mut();
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PATCH, DELETE, OPTIONS"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(
                "authorization, content-type, accept, mcp-session-id, mcp-protocol-version, last-event-id",
            ),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        r
    } else {
        next.run(req).await
    };
    let h = resp.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, allow);
    h.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("mcp-session-id, content-disposition"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Origin"));
    resp
}

async fn limit_body(State(st): St, req: Request, next: Next) -> Response {
    let max = if is_upload(req.uri().path()) {
        st.opts.max_upload_bytes
    } else {
        st.opts.max_body_bytes
    };
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > max as u64) {
        return ApiError::too_large(format!("request body is larger than {max} bytes"))
            .into_response();
    }
    let (parts, body) = req.into_parts();
    let limited = axum::body::Body::new(http_body_util::Limited::new(body, max));
    next.run(Request::from_parts(parts, limited)).await
}

async fn require_token(State(st): St, req: Request, next: Next) -> Response {
    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(t) if st.opts.token.verify(t) => next.run(req).await,
        Some(_) => ApiError::unauthorized("wrong bearer token").into_response(),
        None => ApiError::unauthorized("missing bearer token").into_response(),
    }
}
