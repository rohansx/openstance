//! P4-API-02 / P4-API-04 acceptance and the security review checklist
//! (security-and-threat-model.md section 5): bearer token on every route but
//! `/health` and the static UI, Host check (421), CORS allow-list only,
//! security headers, body limits.

mod common;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use common::*;

#[tokio::test]
async fn health_needs_no_token_and_reports_versions() {
    let h = harness();
    let (status, headers, body) = h
        .send(
            req(Method::GET, "/health")
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["schema_version"], openstance_core::db::LATEST_VERSION);
    assert_eq!(headers["content-type"], "application/json; charset=utf-8");
}

#[tokio::test]
async fn missing_or_wrong_token_is_401_everywhere_else() {
    let h = harness();
    for uri in [
        "/people?q=x",
        "/people/2",
        "/stats",
        "/shortlist",
        "/config",
        "/review",
    ] {
        let (status, headers, body) = h
            .send(
                req(Method::GET, uri)
                    .header("host", HOST)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(headers["www-authenticate"], "Bearer");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["error"]["code"], "unauthorized");

        let (status, _, _) = h
            .send(
                req(Method::GET, uri)
                    .header("host", HOST)
                    .header("authorization", "Bearer not-the-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    // Other schemes and a bare token are refused too.
    for auth in [
        format!("Basic {TOKEN}"),
        TOKEN.to_string(),
        format!("bearer  {TOKEN}x"),
    ] {
        let (status, _, _) = h
            .send(
                req(Method::GET, "/stats")
                    .header("host", HOST)
                    .header("authorization", auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn mcp_needs_the_token() {
    let h = harness();
    let (status, _, _) = h
        .send(
            req(Method::POST, "/mcp")
                .header("host", HOST)
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_right_token_works() {
    let h = harness();
    let (status, v) = h.get("/stats").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["v"], 1);
}

#[tokio::test]
async fn foreign_host_headers_are_421() {
    let h = harness();
    for host in [
        "attacker.example",
        "attacker.example:7411",
        "127.0.0.1:9999",
        "localhost",
        "",
    ] {
        let mut b = req(Method::GET, "/health");
        if !host.is_empty() {
            b = b.header("host", host);
        }
        let (status, _, body) = h.send(b.body(Body::empty()).unwrap()).await;
        assert_eq!(status.as_u16(), 421, "{host}");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "misdirected");
    }
    // Even with a valid token.
    let (status, _, _) = h
        .send(
            req(Method::GET, "/stats")
                .header("host", "rebind.attacker.example:7411")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status.as_u16(), 421);
    for host in ["127.0.0.1:7411", "localhost:7411", "[::1]:7411"] {
        let (status, _, _) = h
            .send(
                req(Method::GET, "/health")
                    .header("host", host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn a_foreign_origin_is_refused_and_gets_no_cors_headers() {
    let h = harness();
    let (status, headers, _) = h
        .send(
            req(Method::GET, "/stats")
                .header("host", HOST)
                .header("origin", "https://evil.example")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(headers
        .keys()
        .all(|k| !k.as_str().starts_with("access-control-")));
    // Preflight from the same origin: no allow headers either.
    let (status, headers, _) = h
        .send(
            req(Method::OPTIONS, "/edits")
                .header("host", HOST)
                .header("origin", "https://evil.example")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization, content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(headers
        .keys()
        .all(|k| !k.as_str().starts_with("access-control-")));
}

#[tokio::test]
async fn same_origin_requests_from_the_embedded_ui_pass_without_cors_headers() {
    let h = harness();
    let (status, headers, _) = h
        .send(
            req(Method::POST, "/shortlist")
                .header("host", HOST)
                .header("origin", "http://127.0.0.1:7411")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"person": 2}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .keys()
        .all(|k| !k.as_str().starts_with("access-control-")));
}

#[tokio::test]
async fn a_listed_origin_gets_cors_headers_and_a_preflight() {
    let mut h = harness();
    h.opts.cors_origins = vec!["http://localhost:5173".into()];
    let (status, headers, _) = h
        .send(
            req(Method::OPTIONS, "/edits")
                .header("host", HOST)
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization, content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        headers["access-control-allow-origin"],
        "http://localhost:5173"
    );
    let methods = headers["access-control-allow-methods"].to_str().unwrap();
    assert!(
        methods.contains("PATCH") && methods.contains("DELETE"),
        "{methods}"
    );
    let allow = headers["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(allow.contains("authorization") && allow.contains("content-type"));
    assert!(!headers.contains_key("access-control-allow-credentials"));
    assert_eq!(headers["vary"], "Origin");

    let (status, headers, _) = h
        .send(
            req(Method::GET, "/stats")
                .header("host", HOST)
                .header("origin", "http://localhost:5173")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["access-control-allow-origin"],
        "http://localhost:5173"
    );
    // A listed origin still needs the token.
    let (status, headers, _) = h
        .send(
            req(Method::GET, "/stats")
                .header("host", HOST)
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        headers["access-control-allow-origin"],
        "http://localhost:5173"
    );
}

#[tokio::test]
async fn api_responses_carry_the_security_headers() {
    let h = harness();
    let (_, headers, _) = h
        .send(
            req(Method::GET, "/stats")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["x-frame-options"], "DENY");
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("default-src 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(!headers.contains_key("set-cookie"));
}

#[tokio::test]
async fn bodies_over_one_megabyte_are_413() {
    let h = harness();
    let note = "x".repeat(1024 * 1024 + 10);
    let body = serde_json::json!({ "person": 2, "note": note }).to_string();
    let (status, _, bytes) = h
        .send(
            req(Method::POST, "/shortlist")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn unknown_routes_and_bad_json_use_the_error_envelope() {
    let h = harness();
    let (status, v) = h.get("/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");
    let (status, _, bytes) = h
        .send(
            req(Method::POST, "/edits")
                .header("host", HOST)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["code"], "bad_request");
    let (status, v) = h.call(Method::DELETE, "/stats", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{v}");
    assert_eq!(v["error"]["code"], "method_not_allowed");
}

#[tokio::test]
async fn mcp_only_serves_health_and_mcp() {
    let mut h = harness();
    h.opts.api = false;
    let (status, _) = h.get("/stats").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = h
        .send(
            req(Method::GET, "/health")
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}
