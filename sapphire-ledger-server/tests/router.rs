//! The composition this branch exists to add: `/mcp`, and only `/mcp`,
//! behind the framework's key check.
//!
//! `protect`'s own semantics are the framework's to test, and they are
//! tested there. What no test in this repository covered until now is the
//! wiring: that `build_state` still calls `.with_keys(...)`, that nobody has
//! reached for `ServerState::insecure_for_tests()`, and that `/rpc` has not
//! quietly become reachable. Each of those is a one-line mistake that leaves
//! every other test on this branch passing.
//!
//! These drive [`serve::build_router`] directly with
//! `tower::ServiceExt::oneshot` rather than binding a socket, so there is no
//! port to pick and no server to shut down.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;

use sapphire_ledger_server::cli::{Command, DeviceCommand, UserCommand};
use sapphire_ledger_server::{identity, serve};

/// Keep this test binary's cache out of the real user cache directory.
///
/// `set_cache_dir` is first-writer-wins on a process global and these tests
/// share a process, so the `OnceLock` is what makes them agree rather than
/// race.
fn init_test_cache() {
    static CACHE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = CACHE.get_or_init(|| tempfile::tempdir().expect("cache tempdir"));
    sapphire_ledger_core::LEDGER_CTX.set_cache_dir(dir.path().to_path_buf());
}

struct Fixture {
    _dir: tempfile::TempDir,
    ledger: PathBuf,
    keys: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        init_test_cache();
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = dir.path().join("ledger");
        sapphire_ledger_core::init_workspace(&ledger, "JPY").expect("init");
        let keys = dir.path().join("keys.toml");
        Self {
            _dir: dir,
            ledger,
            keys,
        }
    }

    fn ledger_dir(&self) -> &Path {
        &self.ledger
    }

    /// Mint a token the way an operator does, through `device add` -- not by
    /// calling `KeyStore::generate` directly. If the CLI ever stops setting a
    /// device id, or stops writing to this file, these tests should feel it.
    fn mint_token(&self, device: &str) -> String {
        identity::run(
            Command::User(UserCommand::Add {
                name: "me".to_string(),
                description: None,
            }),
            &self.ledger,
            &self.keys,
        )
        .expect("user add");
        identity::run(
            Command::Device(DeviceCommand::Add {
                name: device.to_string(),
                user: "me".to_string(),
                description: None,
                expires_in: None,
            }),
            &self.ledger,
            &self.keys,
        )
        .expect("device add");

        let store = sapphire_framework::remote_server::KeyStore::load(&self.keys).expect("keys");
        store
            .entries()
            .iter()
            .find(|k| k.label.as_deref() == Some(device))
            .expect("device add minted a key")
            .token
            .clone()
    }

    fn router(&self) -> axum::Router {
        let state = serve::build_state(&self.keys).expect("state");
        serve::build_router(
            state,
            self.ledger_dir(),
            &[],
            tokio_util::sync::CancellationToken::new(),
        )
        .expect("router")
    }
}

/// Send one request and report its status. `Host: localhost` because rmcp's
/// allowlist is loopback-only here, and a 403 from that guard would be
/// indistinguishable from the auth answers these tests are about.
async fn status(router: axum::Router, uri: &str, bearer: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().uri(uri).header("host", "localhost");
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = builder.body(Body::empty()).expect("request");
    let response = router.oneshot(request).await.expect("response");
    let status = response.status();
    // Drain the body so a handler that streams cannot leave the response
    // half-built, and so a failure has something to print.
    let _ = response.into_body().collect().await;
    status
}

#[tokio::test]
async fn mcp_without_a_bearer_is_refused() {
    let f = Fixture::new();
    assert_eq!(
        status(f.router(), "/mcp", None).await,
        StatusCode::UNAUTHORIZED,
        "an unauthenticated /mcp request must not reach the ledger -- this is \
         the assertion that catches `protect` being dropped from `build_router`, \
         `build_state` losing `.with_keys(..)`, or someone reaching for \
         `ServerState::insecure_for_tests()`"
    );
}

#[tokio::test]
async fn mcp_with_a_bogus_bearer_is_refused() {
    let f = Fixture::new();
    let _real = f.mint_token("laptop");
    assert_eq!(
        status(f.router(), "/mcp", Some("sl_not_a_real_token")).await,
        StatusCode::UNAUTHORIZED,
        "a token that names no key must be refused even when the store holds \
         a valid one"
    );
}

#[tokio::test]
async fn mcp_with_a_minted_bearer_gets_past_the_key_check() {
    let f = Fixture::new();
    let token = f.mint_token("laptop");
    let status = status(f.router(), "/mcp", Some(&token)).await;
    // What rmcp answers a bare GET is rmcp's business -- it may be 405, 406
    // or a stream. The claim here is only that the key check let it through,
    // which is what makes the 401s above evidence of authentication rather
    // than of a router that refuses everything.
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "a token minted by `device add` must authenticate"
    );
    assert_ne!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "503 is `protect`'s no-key-store-configured refusal; the state built \
         here does have one"
    );
}

#[tokio::test]
async fn rpc_is_not_mounted() {
    let f = Fixture::new();
    let token = f.mint_token("laptop");
    // Authenticated on purpose: `protect` layers the whole router, so an
    // unauthenticated /rpc would 401 before routing and tell us nothing about
    // whether the route exists.
    assert_eq!(
        status(f.router(), "/rpc", Some(&token)).await,
        StatusCode::NOT_FOUND,
        "`/rpc` is deliberately absent -- the framework's `router()` must not \
         be merged while sync has no client. When sync lands, this should \
         become a real route test rather than being deleted."
    );
}

#[tokio::test]
async fn a_state_with_no_key_store_refuses_everything() {
    // The complement of the tests above: `build_state` always attaches a key
    // store, and if it ever stopped, `protect` would close rather than open.
    // Asserting that here means a regression shows up as a 503 rather than as
    // a silently unauthenticated `/mcp`.
    let f = Fixture::new();
    let state = Arc::new(sapphire_framework::remote_server::ServerState::new(
        f.ledger_dir().to_path_buf(),
    ));
    let router = serve::build_router(
        state,
        f.ledger_dir(),
        &[],
        tokio_util::sync::CancellationToken::new(),
    )
    .expect("router");
    assert_eq!(
        status(router, "/mcp", None).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "no key store configured must close the router, never open it"
    );
}
