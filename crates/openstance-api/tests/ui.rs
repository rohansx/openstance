//! P4-API-06: the embedded UI is served without a token (it holds no data;
//! every API call it makes carries the token from the `#token=` fragment),
//! and `--no-ui` turns it off.

mod common;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use common::*;

#[tokio::test]
async fn root_serves_html_without_a_token() {
    let h = harness();
    let (status, headers, body) = h
        .send(
            req(Method::GET, "/")
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let html = String::from_utf8(body).unwrap();
    assert!(
        html.contains("<html") || html.contains("<!doctype html"),
        "{html}"
    );
    assert!(!html.contains(TOKEN));
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("connect-src 'self'"));
}

#[tokio::test]
async fn root_still_checks_the_host() {
    let h = harness();
    let (status, _, _) = h
        .send(
            req(Method::GET, "/")
                .header("host", "evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status.as_u16(), 421);
}

#[tokio::test]
async fn unknown_assets_are_404_and_spa_routes_fall_back_to_the_app() {
    let h = harness();
    let (status, _, _) = h
        .send(
            req(Method::GET, "/assets/does-not-exist.js")
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, headers, _) = h
        .send(
            req(Method::GET, "/app/people/2")
                .header("host", HOST)
                .header("accept", "text/html,application/xhtml+xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
}

#[tokio::test]
async fn no_ui_turns_static_files_off() {
    let mut h = harness();
    h.opts.ui = false;
    let (status, _, _) = h
        .send(
            req(Method::GET, "/")
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn placeholder_explains_how_to_build_the_ui() {
    let html = openstance_api::ui::PLACEHOLDER;
    assert!(html.contains("npm run build"));
    assert!(html.contains("ui/"));
}
