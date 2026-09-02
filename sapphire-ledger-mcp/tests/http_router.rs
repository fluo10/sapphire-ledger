//! The `Host` allowlist is the quiet failure mode: widen the bind without
//! widening the list and every MCP request 403s while the process looks
//! healthy. These tests exist so that stays impossible to ship.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;

/// The `TempDir` is held for the whole request rather than leaked: two of
/// these cases pass the `Host` check and reach the handler, which needs the
/// workspace to still be on disk.
async fn status_for_host(allowed: &[String], host: &str) -> StatusCode {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = sapphire_ledger_mcp::server::prepare_state(Some(dir.path()), true)
        .expect("init test ledger");
    let router = sapphire_ledger_mcp::http::mcp_router(
        Arc::new(Mutex::new(state)),
        CancellationToken::new(),
        None,
        allowed,
    );
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", host)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
        .expect("request");
    let status = router.oneshot(request).await.expect("response").status();
    drop(dir);
    status
}

#[tokio::test]
async fn loopback_is_allowed_with_an_empty_configured_list() {
    assert_ne!(
        status_for_host(&[], "127.0.0.1:3838").await,
        StatusCode::FORBIDDEN,
        "loopback must always be allowed; an empty list must never mean \
         'allow nothing', and must never be passed through to rmcp as \
         'allow everything' either"
    );
}

#[tokio::test]
async fn a_configured_host_is_allowed() {
    let allowed = vec!["ledger.example.net".to_string()];
    assert_ne!(
        status_for_host(&allowed, "ledger.example.net").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn an_unlisted_host_is_refused() {
    let allowed = vec!["ledger.example.net".to_string()];
    assert_eq!(
        status_for_host(&allowed, "evil.example.com").await,
        StatusCode::FORBIDDEN,
        "the allowlist is the DNS-rebinding defence; an unlisted Host must 403"
    );
}

#[tokio::test]
async fn a_blank_configured_host_does_not_disable_the_guard() {
    // rmcp reads an *empty allowlist* as "allow every host". A blank string
    // in the config must be filtered out rather than widening the list to
    // something meaningless — and must not empty it either.
    let allowed = vec!["   ".to_string()];
    assert_eq!(
        status_for_host(&allowed, "evil.example.com").await,
        StatusCode::FORBIDDEN
    );
}
