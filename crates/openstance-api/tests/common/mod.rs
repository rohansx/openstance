//! Shared test harness: a temp home with the fictional search test kit
//! seeded (people 1..7, NOW = 2026-10-20T12:00:00Z) and the router built
//! over it, driven in-process with `tower::ServiceExt::oneshot`.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use openstance_api::pool::{ClockFn, GraphPool};
use openstance_api::token::Token;
use openstance_api::{app, ServeOptions};
use openstance_core::search::testkit::{self, NOW};
use openstance_core::{Clock, Config, FixedClock, Providers};
use serde_json::Value;
use tower::ServiceExt;

pub const TOKEN: &str = "test-token-0123456789abcdefghijklmnopqrstuv";
pub const PORT: u16 = 7411;
pub const HOST: &str = "127.0.0.1:7411";

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub pool: GraphPool,
    pub opts: ServeOptions,
}

pub fn clock() -> ClockFn {
    Arc::new(|| Box::new(FixedClock(NOW)) as Box<dyn Clock>)
}

pub fn options(dir: &std::path::Path) -> ServeOptions {
    ServeOptions {
        port: PORT,
        token: Token::from_secret(TOKEN),
        share: Some(openstance_core::view::ShareDirs {
            key: dir.join("age.key"),
            views: dir.join("views"),
        }),
        owner_label: Some("tester".into()),
        config_doc: serde_json::json!({ "v": 1, "config": {} }),
        ..ServeOptions::default()
    }
}

/// A seeded, FTS-indexed graph in a temp dir.
pub fn harness_with(cfg: Config, providers: Providers) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("graph.db");
    {
        let mut c = openstance_core::db::open(&db, &Default::default()).unwrap();
        openstance_core::db::migrate(&mut c, Some(&db), 0).unwrap();
        testkit::seed(&c);
        openstance_core::search::fts::reindex_all(&c).unwrap();
    }
    let pool = GraphPool::open(cfg, &db, providers, clock(), 2).unwrap();
    let opts = options(dir.path());
    Harness { dir, pool, opts }
}

pub fn harness() -> Harness {
    harness_with(Config::default(), Providers::null())
}

impl Harness {
    pub fn router(&self) -> Router {
        app(self.pool.clone(), self.opts.clone())
    }

    pub async fn send(&self, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let resp = self.router().oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    /// An authorized request; returns status and parsed JSON.
    pub async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", HOST)
            .header("authorization", format!("Bearer {TOKEN}"));
        let body = match body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&v).unwrap())
            }
            None => Body::empty(),
        };
        let (status, _, bytes) = self.send(b.body(body).unwrap()).await;
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, v)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::GET, uri, None).await
    }

    /// Run `f` on a reader (direct core calls to compare against).
    pub async fn core<T: Send + 'static>(
        &self,
        f: impl FnOnce(&openstance_core::Graph) -> T + Send + 'static,
    ) -> T {
        self.pool.read(move |g| Ok(f(g))).await.unwrap()
    }
}

pub fn req(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(uri)
}
