//! The embedded web UI (P4-API-06). With the `ui` feature and a built
//! `ui/dist` (detected by `build.rs`), the files are compiled into the
//! binary; otherwise `/` serves [`PLACEHOLDER`]. The static files are public
//! (they hold no data); every API call the UI makes carries the token it
//! read from the `#token=` fragment and keeps in `sessionStorage`.

use std::borrow::Cow;

use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

/// Content-Security-Policy on every response. Everything is same-origin;
/// inline style attributes and `data:` images are allowed for the UI, never
/// inline or remote scripts.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// Served at `/` when this binary was built without the UI.
pub const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>openstance</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 38rem; margin: 4rem auto; padding: 0 1rem; }
  h1 { font-family: Georgia, "Times New Roman", serif; font-weight: 500; }
  code { background: rgba(127,127,127,.15); padding: .1rem .3rem; border-radius: 4px; }
</style>
</head>
<body>
<h1>openstance is running</h1>
<p>The HTTP API and MCP endpoint are up, but this binary was built without the web UI.</p>
<p>To add it, build the UI and then rebuild openstance with the <code>ui</code> feature:</p>
<pre><code>cd ui/
npm ci
npm run build
cd ..
cargo build --release --features ui</code></pre>
<p>The API is documented in <code>docs/tech-spec/interfaces.md</code>. Every route except <code>/health</code> needs the bearer token from <code>~/.openstance/api.token</code>.</p>
</body>
</html>
"#;

#[cfg(all(feature = "ui", ui_dist))]
#[derive(rust_embed::Embed)]
#[folder = "../../ui/dist"]
struct Assets;

/// True when this binary carries the built UI.
pub fn embedded() -> bool {
    cfg!(all(feature = "ui", ui_dist))
}

/// An embedded file and its MIME type.
fn file(path: &str) -> Option<(Cow<'static, [u8]>, String)> {
    #[cfg(all(feature = "ui", ui_dist))]
    {
        Assets::get(path).map(|f| (f.data, f.metadata.mimetype().to_string()))
    }
    #[cfg(not(all(feature = "ui", ui_dist)))]
    {
        let _ = path;
        None
    }
}

fn index() -> Response {
    let (body, _) = file("index.html").unwrap_or((
        Cow::Borrowed(PLACEHOLDER.as_bytes()),
        "text/html".to_string(),
    ));
    let mut r = (StatusCode::OK, body.into_owned()).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

/// A request for a static UI file (only used to pick cache headers).
pub fn is_ui_request(req: &Request) -> bool {
    let p = req.uri().path();
    req.method() == Method::GET && (p == "/" || p.starts_with("/assets/"))
}

/// The router fallback: static files, `index.html` for app routes that a
/// browser navigates to, the error envelope for everything else.
pub async fn serve(req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return ApiError::not_found("no such route").into_response();
    }
    let path = req.uri().path();
    if path == "/" || path == "/index.html" {
        return index();
    }
    let rel = path.trim_start_matches('/');
    if !rel.contains("..") {
        if let Some((body, mime)) = file(rel) {
            let mut r = (StatusCode::OK, body.into_owned()).into_response();
            if let Ok(v) = HeaderValue::from_str(&mime) {
                r.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            // Vite puts a content hash in every file name under assets/.
            let cache = if rel.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            r.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
            return r;
        }
    }
    let last = rel.rsplit('/').next().unwrap_or_default();
    let wants_html = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|a| a.to_str().ok())
        .is_some_and(|a| a.contains("text/html"));
    if wants_html && !rel.starts_with("assets/") && !last.contains('.') {
        return index();
    }
    ApiError::not_found("no such route").into_response()
}
