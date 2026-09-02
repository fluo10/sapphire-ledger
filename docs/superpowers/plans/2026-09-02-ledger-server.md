# sapphire-ledger-server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One self-hosted binary serving the ledger's MCP surface over HTTP, authenticated per device, so an agent in another process or on another machine can read and record entries.

**Architecture:** `sapphire-ledger-mcp` gains an `http-server` feature exposing `mcp_router`. A new `sapphire-ledger-server` crate builds a `ServerState` carrying only a `KeyStore`, wraps that router in the framework's `protect()`, and serves it. `/rpc` is not mounted. Clients are devices belonging to users, both held in the framework's registry; a key names its device.

**Tech Stack:** Rust 2024, `rmcp` 1.5 (`transport-streamable-http-server`), `axum` 0.8, `tokio`, `tokio-util`, `sapphire-framework` (`remote-server` + `registry`), `grain-id`, `clap`.

**Spec:** [`docs/superpowers/specs/2026-09-02-ledger-server-design.md`](../specs/2026-09-02-ledger-server-design.md)

## Global Constraints

- Rust edition 2024, workspace resolver `3`; every crate `MIT OR Apache-2.0` via `license.workspace = true`.
- **`/rpc` is not mounted.** Take `sapphire-framework`'s `remote-server` feature for `KeyStore`, `protect`, `Authenticated` and `ServerState` — never `.merge()` its `router()`.
- **Registry files live in the workspace** (`Workspace::devices_path()` / `users_path()`); **the key file lives in the host-local cache directory.** The registry is content that must sync; the token is a secret that must not.
- **Ids are `grain-id`** for devices and users, matching ledger's record ids. `KeyEntry.id` stays a `Uuid` — that is the framework's type, not ours to change.
- **There is no standalone `gen-key`.** Every key names a device.
- **`stdout` is a machine contract.** `device add` and `device rotate` print the raw token and nothing else; every log line and warning goes to `stderr`. On Windows the framework emits a permissions warning when creating the key file, and it must not land in `device add > token.txt`.
- **Loopback by default.** `--addr` widens the bind, `--allowed-host` (repeatable) widens rmcp's `Host` allowlist. **Bound beyond loopback with no `--allowed-host`, refuse to start.**
- **Duplicate ids are detected and reported, never resolved.**
- No `rusqlite`, no SQLite cache. Do not add a `dedupe` module.
- Run `cargo fmt` over files you touch and nothing else. CI gates `fmt --check`, `clippy -D warnings` and `test --locked` on the pinned toolchain.

---

## File Structure

**`sapphire-ledger-mcp/`**
- `Cargo.toml` — add the `http-server` feature and its optional deps.
- `src/http.rs` — **new**, feature-gated. `mcp_router` only.
- `src/lib.rs` — declare and re-export it under the feature.

**`sapphire-ledger-server/`** — new crate
- `Cargo.toml`
- `src/lib.rs` — declares the three modules so integration tests can reach them.
- `src/main.rs` — clap entry, tracing to stderr, dispatch.
- `src/cli.rs` — the command tree.
- `src/serve.rs` — path helpers, state construction, bind, graceful shutdown, the refuse-to-start rule.
- `src/identity.rs` — the user/device CLI over the registry and `KeyStore`.
- `src/watch.rs` — the duplicate-id check and its tick.

**Workspace `Cargo.toml`** — add the member and the shared deps.

---

### Task 1: `http-server` feature and `mcp_router`

**Files:**
- Modify: `sapphire-ledger-mcp/Cargo.toml`
- Create: `sapphire-ledger-mcp/src/http.rs`
- Modify: `sapphire-ledger-mcp/src/lib.rs`
- Test: `sapphire-ledger-mcp/tests/http_router.rs`

**Interfaces:**
- Consumes: `SapphireLedgerServer::from_shared(Arc<Mutex<LedgerState>>)`, `::with_write_observer(WriteObserver)`, `server::prepare_state(Option<&Path>, bool)` — all from the stdio work already merged.
- Produces: `sapphire_ledger_mcp::http::mcp_router(shared_state: Arc<Mutex<LedgerState>>, cancel: CancellationToken, observer: Option<WriteObserver>, allowed_hosts: &[String]) -> axum::Router`, re-exported as `sapphire_ledger_mcp::mcp_router`.

- [ ] **Step 1: Add the feature and deps**

In `sapphire-ledger-mcp/Cargo.toml`, extend `[features]`:

```toml
http-server = ["rmcp/transport-streamable-http-server", "dep:axum", "dep:tokio-util"]
```

and add to `[dependencies]`:

```toml
axum = { version = "0.8", default-features = false, features = ["http1", "tokio"], optional = true }
tokio-util = { version = "0.7", optional = true }
```

and to `[dev-dependencies]`:

```toml
axum = { version = "0.8", default-features = false, features = ["http1", "tokio"] }
tower = { version = "0.5", features = ["util"] }
tokio-util = "0.7"
http-body-util = "0.1"
```

and register the feature-gated test:

```toml
[[test]]
name = "http_router"
path = "tests/http_router.rs"
required-features = ["http-server"]
```

- [ ] **Step 2: Write the failing test**

Create `sapphire-ledger-mcp/tests/http_router.rs`:

```rust
//! The `Host` allowlist is the quiet failure mode: widen the bind without
//! widening the list and every MCP request 403s while the process looks
//! healthy. These tests exist so that stays impossible to ship.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt as _;

fn router(allowed: &[String]) -> axum::Router {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = sapphire_ledger_mcp::server::prepare_state(Some(dir.path()), true)
        .expect("init test ledger");
    // The TempDir is dropped here on purpose: every one of these tests
    // fails at the Host check, before any handler touches the workspace.
    std::mem::forget(dir);
    sapphire_ledger_mcp::http::mcp_router(
        Arc::new(Mutex::new(state)),
        CancellationToken::new(),
        None,
        allowed,
    )
}

async fn status_for_host(allowed: &[String], host: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", host)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
        .expect("request");
    router(allowed)
        .oneshot(request)
        .await
        .expect("response")
        .status()
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
```

- [ ] **Step 3: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-mcp --features http-server --test http_router
```

Expected: FAIL — `sapphire_ledger_mcp::http` does not exist.

- [ ] **Step 4: Implement the router**

Create `sapphire-ledger-mcp/src/http.rs`:

```rust
//! HTTP transport for the MCP server: [`mcp_router`] builds the `/mcp` route
//! for a host that already runs an `axum` server to merge into its own.
//! `sapphire-ledger-server` does that, so the MCP surface and — later — the
//! sync API share one process, one port and one auth layer.
//!
//! ## The `Host` allowlist is not optional
//!
//! rmcp defends against DNS rebinding by refusing any request whose `Host`
//! header is not on an allowlist, and its default list is loopback only
//! (`localhost`, `127.0.0.1`, `::1`). That default is correct for a router
//! bound to loopback and **wrong the moment the host binds anywhere else**: a
//! client reaching the server as `http://ledger.example.net/mcp` sends a
//! `Host` that matches nothing and gets `403 Forbidden`. The process looks
//! healthy the whole time.
//!
//! [`mcp_router`] therefore takes the extra hostnames as an argument instead
//! of leaving them at a default nobody remembers to change. Loopback is always
//! added on top of what the caller passes, so widening the list never costs
//! local use, and passing nothing can never *disable* the guard — an empty
//! list means "allow every host" to rmcp, and this module never hands it one.
//!
//! Authentication is a separate concern this module does not provide. The
//! caller wraps the returned router in the framework's `protect()`.

use std::sync::{Arc, Mutex};

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use sapphire_ledger_core::LedgerState;

use crate::server::{SapphireLedgerServer, WriteObserver};

/// rmcp's default `Host` allowlist: loopback only. [`mcp_router`] adds the
/// caller's hosts *on top of* these rather than replacing them, so widening
/// the bind never breaks local use.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Build an [`axum::Router`] serving MCP at `/mcp`.
///
/// Pass the hostnames clients actually use in `allowed_hosts` — the bind
/// address, a LAN name, a Tailscale name. Loopback is always added, so an
/// empty slice yields exactly rmcp's loopback-only default and never the
/// "allow everything" state rmcp infers from a genuinely empty list.
///
/// This router is **unauthenticated**. Wrap it in the framework's `protect()`.
pub fn mcp_router(
    shared_state: Arc<Mutex<LedgerState>>,
    cancel: CancellationToken,
    observer: Option<WriteObserver>,
    allowed_hosts: &[String],
) -> axum::Router {
    let factory_state = Arc::clone(&shared_state);
    let factory = move || {
        let server = SapphireLedgerServer::from_shared(Arc::clone(&factory_state));
        // A fresh server is built per session, so the observer is re-attached
        // each time rather than held once.
        Ok(match &observer {
            Some(o) => server.with_write_observer(Arc::clone(o)),
            None => server,
        })
    };

    let hosts: Vec<String> = LOOPBACK_HOSTS
        .iter()
        .map(|h| (*h).to_owned())
        .chain(
            allowed_hosts
                .iter()
                .filter(|h| !h.trim().is_empty())
                .cloned(),
        )
        .collect();
    tracing::debug!(?hosts, "MCP Host allowlist");

    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(cancel)
        .with_allowed_hosts(hosts);
    let service = StreamableHttpService::new(
        factory,
        Arc::new(LocalSessionManager::default()),
        config,
    );

    axum::Router::new().route_service("/mcp", service)
}
```

In `sapphire-ledger-mcp/src/lib.rs`, add:

```rust
#[cfg(feature = "http-server")]
pub mod http;

#[cfg(feature = "http-server")]
pub use http::mcp_router;
```

`server` is already `pub mod`, so the test's `sapphire_ledger_mcp::server::prepare_state` resolves without further change.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-mcp --features http-server
cargo test -p sapphire-ledger-mcp
```

Expected: the four new tests pass under the feature, and the existing stdio tests still pass with it off.

- [ ] **Step 6: Commit**

```bash
git add sapphire-ledger-mcp/
git commit -m "feat(mcp): serve the ledger over HTTP behind an explicit Host allowlist

The allowlist is an argument rather than a default because getting it wrong
fails quietly: widen the bind without widening the list and every request
403s while the process looks healthy."
```

---

### Task 2: The server crate, serving `/mcp` behind `protect()`

**Files:**
- Modify: `Cargo.toml` (workspace members and deps)
- Create: `sapphire-ledger-server/Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/cli.rs`, `src/serve.rs`
- Test: `sapphire-ledger-server/tests/serve.rs`

**Interfaces:**
- Consumes: `mcp_router` (Task 1); `sapphire_ledger_mcp::server::prepare_state`.
- Produces:
  - `serve::default_keys_path(ledger_dir: &Path) -> anyhow::Result<PathBuf>`
  - `serve::build_state(keys_path: &Path) -> anyhow::Result<Arc<ServerState>>`
  - `serve::check_exposure(addr: SocketAddr, allowed_hosts: &[String]) -> anyhow::Result<()>`
  - `serve::run(addr: SocketAddr, ledger_dir: &Path, state: Arc<ServerState>, allowed_hosts: &[String]) -> anyhow::Result<()>`
  - `cli::Cli` with `command: Option<Command>`, `ledger_dir: Option<PathBuf>`, `keys: Option<PathBuf>`, `addr: SocketAddr`, `allowed_host: Vec<String>`

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, add `"sapphire-ledger-server"` to `[workspace.members]`, and under `[workspace.dependencies]`:

```toml
axum = { version = "0.8", default-features = false, features = ["http1", "tokio"] }
tokio-util = "0.7"
humantime = "2"
```

- [ ] **Step 2: Write the crate manifest**

Create `sapphire-ledger-server/Cargo.toml`:

```toml
[package]
name = "sapphire-ledger-server"
version = "0.1.0"
edition.workspace = true
description = "Self-hosted MCP server for sapphire-ledger, authenticated per device"
license.workspace = true
repository.workspace = true
categories = ["command-line-utilities", "web-programming::http-server"]
keywords = ["accounting", "ledger", "mcp", "server"]
publish = false

[[bin]]
name = "sapphire-ledger-server"
path = "src/main.rs"

[features]
default = ["redb-store"]
redb-store = ["sapphire-ledger-core/redb-store", "sapphire-ledger-mcp/redb-store"]

[dependencies]
sapphire-ledger-core = { path = "../sapphire-ledger-core", version = "0.1.0", default-features = false }
sapphire-ledger-mcp = { path = "../sapphire-ledger-mcp", version = "0.1.0", default-features = false, features = ["http-server"] }
sapphire-framework = { workspace = true, features = ["remote-server", "registry"] }
grain-id.workspace = true
axum.workspace = true
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "signal", "time"] }
tokio-util.workspace = true
clap.workspace = true
anyhow.workspace = true
chrono.workspace = true
humantime.workspace = true
tracing.workspace = true
tracing-subscriber = { workspace = true, features = ["env-filter"] }

[dev-dependencies]
tempfile.workspace = true
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

The workspace `sapphire-framework` entry currently has `default-features = false` and no features; adding `features = [...]` here is additive and does not disturb `sapphire-ledger-core`'s use of it.

- [ ] **Step 3: Write the failing test**

Create `sapphire-ledger-server/tests/serve.rs`:

```rust
use std::net::SocketAddr;

use sapphire_ledger_server::serve;

fn addr(s: &str) -> SocketAddr {
    s.parse().expect("addr")
}

#[test]
fn loopback_needs_no_allowed_host() {
    serve::check_exposure(addr("127.0.0.1:3838"), &[])
        .expect("a loopback bind is complete on its own");
}

#[test]
fn a_wide_bind_without_an_allowed_host_is_refused() {
    let err = serve::check_exposure(addr("0.0.0.0:3838"), &[])
        .expect_err("a wide bind with no allowlist serves 403 to everyone");
    let msg = err.to_string();
    assert!(
        msg.contains("--allowed-host"),
        "the error must name the flag that fixes it, got: {msg}"
    );
}

#[test]
fn a_wide_bind_with_an_allowed_host_is_accepted() {
    serve::check_exposure(addr("0.0.0.0:3838"), &["ledger.example.net".to_string()])
        .expect("a named host makes a wide bind usable");
}

#[test]
fn a_blank_allowed_host_does_not_count() {
    serve::check_exposure(addr("0.0.0.0:3838"), &["  ".to_string()])
        .expect_err("whitespace is not a hostname");
}

#[test]
fn state_without_a_key_file_still_builds_but_holds_no_keys() {
    // The framework's `protect()` refuses every request when no key store is
    // configured, rather than passing them through. Building the state must
    // not be what fails — the refusal has to happen at request time, where an
    // operator can see it.
    let dir = tempfile::tempdir().expect("tempdir");
    let state = serve::build_state(&dir.path().join("keys.toml")).expect("state builds");
    assert!(state.keys().is_some(), "an empty key store is still a key store");
}
```

- [ ] **Step 4: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-server --test serve
```

Expected: FAIL — the crate does not compile; `serve` does not exist.

- [ ] **Step 5: Write `cli.rs`**

Create `sapphire-ledger-server/src/cli.rs`:

```rust
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sapphire-ledger-server",
    about = "Self-hosted MCP server for sapphire-ledger",
    version
)]
pub struct Cli {
    /// Path to the ledger root (the directory containing `.sapphire-ledger/`).
    #[arg(long, env = "SAPPHIRE_LEDGER_SERVER_DIR", global = true, value_name = "DIR")]
    pub ledger_dir: Option<PathBuf>,

    /// Key file. Defaults to `keys.toml` in this app's cache directory.
    #[arg(long, global = true, value_name = "FILE")]
    pub keys: Option<PathBuf>,

    /// Address to bind. Loopback by default; widening it requires
    /// `--allowed-host`.
    #[arg(long, default_value = "127.0.0.1:3838")]
    pub addr: SocketAddr,

    /// A hostname clients use to reach this server. Repeatable. Loopback is
    /// always allowed and never needs listing.
    #[arg(long, value_name = "HOST")]
    pub allowed_host: Vec<String>,

    /// Omit to serve.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage the people devices belong to.
    #[command(subcommand)]
    User(UserCommand),
    /// Manage the clients that may reach this server.
    #[command(subcommand)]
    Device(DeviceCommand),
}

#[derive(Subcommand)]
pub enum UserCommand {
    /// Register a person or an agent.
    Add {
        name: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// List users.
    List,
}

#[derive(Subcommand)]
pub enum DeviceCommand {
    /// Register a device and mint its token. The token is printed once, to
    /// stdout, and is not recoverable afterwards.
    Add {
        name: String,
        /// The user this device belongs to, by name or id.
        #[arg(long)]
        user: String,
        #[arg(long)]
        description: Option<String>,
        /// Expire the token after this long, e.g. `90d`, `12h`.
        #[arg(long, value_name = "DURATION")]
        expires_in: Option<String>,
    },
    /// List devices, with their user and their token masked.
    List,
    /// Re-issue a device's token. The device keeps its id.
    ///
    /// This REPLACES the expiry rather than carrying the old one over:
    /// omitting the flag makes the new token non-expiring.
    Rotate {
        /// The device, by name or id.
        selector: String,
        #[arg(long, value_name = "DURATION")]
        expires_in: Option<String>,
    },
    /// Retire a device: tombstone it and revoke its key. The id still
    /// resolves to a name afterwards, so records that named it stay readable.
    Retire {
        /// The device, by name or id.
        selector: String,
    },
}
```

- [ ] **Step 6: Write `serve.rs`**

Create `sapphire-ledger-server/src/serve.rs`:

```rust
//! Building and running the HTTP server.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use sapphire_framework::remote_server::{KeyStore, ServerState, protect};
use tokio_util::sync::CancellationToken;

/// Where the key file goes when `--keys` is not given: this app's cache
/// directory, **not** the workspace.
///
/// The workspace is what syncs once `/rpc` exists, and a token that syncs is
/// a token on every machine that ever pulled. The device and user tables are
/// the opposite case and do live in the workspace — they are content, and a
/// device id is only resolvable elsewhere if they travel.
pub fn default_keys_path(ledger_dir: &Path) -> anyhow::Result<PathBuf> {
    let _ = ledger_dir;
    let cache = sapphire_ledger_core::LEDGER_CTX
        .cache_dir()
        .context("failed to resolve the cache directory for the key file")?;
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("failed to create {}", cache.display()))?;
    Ok(cache.join("keys.toml"))
}

/// Build the server state. Only the key store is configured: no resolver and
/// no workspace store, because `/rpc` is not mounted.
///
/// `data_dir` is where the framework would keep per-workspace sync state. It
/// is required by the constructor and unused here; the cache directory is the
/// honest place for something that will never be written.
pub fn build_state(keys_path: &Path) -> anyhow::Result<Arc<ServerState>> {
    let store = KeyStore::load(keys_path)
        .with_context(|| format!("failed to open the key file {}", keys_path.display()))?;
    let data_dir = keys_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(Arc::new(
        ServerState::new(data_dir).with_keys(Arc::new(store)),
    ))
}

/// Refuse a configuration that would serve 403 to every client.
///
/// rmcp's `Host` allowlist is loopback-only unless widened. Binding beyond
/// loopback without naming a host is not dangerous — it is useless, and it
/// presents as a client bug. Failing here turns that into an obvious
/// configuration error.
///
/// The journal's server only warns in this situation, and that is right
/// there: its `/rpc` keeps answering, so the process is still useful. This
/// one serves nothing but `/mcp`.
pub fn check_exposure(addr: SocketAddr, allowed_hosts: &[String]) -> anyhow::Result<()> {
    if addr.ip().is_loopback() {
        return Ok(());
    }
    if allowed_hosts.iter().any(|h| !h.trim().is_empty()) {
        return Ok(());
    }
    anyhow::bail!(
        "binding to {addr} without --allowed-host would refuse every request: \
         rmcp's Host allowlist stays loopback-only, so a client reaching this \
         server by any other name gets 403. Pass --allowed-host <hostname> for \
         each name clients use, or bind to loopback."
    )
}

/// Serve until Ctrl-C or SIGTERM, then drain.
pub async fn run(
    addr: SocketAddr,
    ledger_dir: &Path,
    state: Arc<ServerState>,
    allowed_hosts: &[String],
) -> anyhow::Result<()> {
    check_exposure(addr, allowed_hosts)?;

    let ledger_state = sapphire_ledger_mcp::server::prepare_state(Some(ledger_dir), false)?;
    let shared = Arc::new(std::sync::Mutex::new(ledger_state));

    let cancel = CancellationToken::new();
    let mcp = sapphire_ledger_mcp::http::mcp_router(
        Arc::clone(&shared),
        cancel.clone(),
        None,
        allowed_hosts,
    );
    let app = protect(Arc::clone(&state), mcp);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!("sapphire-ledger-server listening on http://{addr}/mcp");

    let shutdown = cancel.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown.cancel();
        })
        .await;

    result.context("server failed")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
```

- [ ] **Step 7: Write `lib.rs` and `main.rs`**

Create `sapphire-ledger-server/src/lib.rs`:

```rust
//! Self-hosted MCP server for sapphire-ledger.

pub mod cli;
pub mod serve;
```

Create `sapphire-ledger-server/src/main.rs`:

```rust
use clap::Parser as _;
use sapphire_ledger_server::cli::Cli;
use sapphire_ledger_server::serve;

const LEDGER_DIR_REQUIRED: &str =
    "--ledger-dir is required (or set SAPPHIRE_LEDGER_SERVER_DIR)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // stdout carries the raw token from `device add` and `device rotate`
        // and nothing else, so every log line goes to stderr. On Windows the
        // framework warns when it cannot restrict the key file's permissions,
        // and that warning must not land in `device add > token.txt`.
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let ledger_dir = cli
        .ledger_dir
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{LEDGER_DIR_REQUIRED}"))?;
    let keys_path = match cli.keys.clone() {
        Some(p) => p,
        None => serve::default_keys_path(&ledger_dir)?,
    };

    match cli.command {
        None => {
            let state = serve::build_state(&keys_path)?;
            serve::run(cli.addr, &ledger_dir, state, &cli.allowed_host).await
        }
        Some(_) => anyhow::bail!("user/device commands arrive in the next task"),
    }
}
```

- [ ] **Step 8: Run the tests**

```bash
cargo test -p sapphire-ledger-server
cargo clippy -p sapphire-ledger-server --all-targets -- -D warnings
```

Expected: five tests pass, clippy clean.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock sapphire-ledger-server/
git commit -m "feat(server): serve /mcp behind the framework's key check

ServerState carries only a KeyStore: no resolver, no workspace store, and
router() is never merged, so /rpc is absent rather than disabled.

Refuses to start when bound beyond loopback with no --allowed-host. That
configuration is not dangerous, it is useless -- every request 403s while the
process looks healthy -- so it fails where an operator can see it."
```

---

### Task 3: Users and devices

**Files:**
- Create: `sapphire-ledger-server/src/identity.rs`
- Modify: `sapphire-ledger-server/src/lib.rs`, `src/main.rs`
- Test: `sapphire-ledger-server/tests/identity.rs`

**Interfaces:**
- Consumes: `cli::{Command, UserCommand, DeviceCommand}` and `serve::default_keys_path` (Task 2).
- Produces: `identity::run(command: Command, ledger_dir: &Path, keys_path: &Path) -> anyhow::Result<()>`, and `identity::parse_duration(s: &str) -> anyhow::Result<chrono::Duration>`.

Framework signatures this task calls, verbatim:

- `Users::load(path: &Path) -> Result<Users>`, `.add(name: &str, description: Option<String>) -> Result<User>`, `.entries() -> &[User]`, `.resolve(selector: &str) -> Result<&User>`
- `Devices::load(path: &Path) -> Result<Devices>`, `.add(name: &str, description: Option<String>, user_id: Option<GrainId>) -> Result<Device>`, `.entries() -> &[Device]`, `.get(id: GrainId) -> Option<&Device>`, `.resolve(selector: &str) -> Result<&Device>`, `.retire(selector: &str) -> Result<Device>`
- `KeyStore::load(path: &Path) -> Result<KeyStore>`, `.generate(prefix: &str, id: Option<Uuid>, device_id: Option<GrainId>, label: Option<String>, expires_at: Option<DateTime<Utc>>) -> Result<KeyEntry>`, `.rotate(prefix: &str, selector: &str, expires_at: Option<DateTime<Utc>>) -> Result<KeyEntry>`, `.revoke(selector: &str) -> Result<KeyEntry>`, `.entries() -> &[KeyEntry]`
- `Workspace::devices_path() -> PathBuf`, `Workspace::users_path() -> PathBuf`
- `Device { id: GrainId, name: String, description: Option<String>, user_id: Option<GrainId>, created_at, retired_at: Option<_> }`, `.is_retired() -> bool`
- `KeyEntry { token: String, id: Uuid, device_id: Option<GrainId>, label: Option<String>, created_at, rotated_at, expires_at }`

**`Devices::add` errors on an id collision rather than retrying** — it says so and asks the caller to try again. Retry up to 8 times before giving up; a `GrainId::random()` collision is astronomically unlikely, but silently propagating that error as "device add failed" would be baffling.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-server/tests/identity.rs`:

```rust
use std::path::{Path, PathBuf};

use sapphire_ledger_server::cli::{Command, DeviceCommand, UserCommand};
use sapphire_ledger_server::identity;

struct Fixture {
    _dir: tempfile::TempDir,
    ledger: PathBuf,
    keys: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = dir.path().join("ledger");
    sapphire_ledger_core::init_workspace(&ledger, "JPY").expect("init");
    let keys = dir.path().join("keys.toml");
    Fixture { _dir: dir, ledger, keys }
}

fn run(f: &Fixture, command: Command) -> anyhow::Result<()> {
    identity::run(command, &f.ledger, &f.keys)
}

fn add_user(f: &Fixture, name: &str) {
    run(f, Command::User(UserCommand::Add {
        name: name.to_string(),
        description: None,
    }))
    .expect("user add");
}

fn add_device(f: &Fixture, name: &str, user: &str) -> anyhow::Result<()> {
    run(f, Command::Device(DeviceCommand::Add {
        name: name.to_string(),
        user: user.to_string(),
        description: None,
        expires_in: None,
    }))
}

fn devices(f: &Fixture) -> sapphire_framework::registry::Devices {
    let ws = sapphire_ledger_core::LEDGER_CTX.workspace_at(&f.ledger).expect("workspace");
    sapphire_framework::registry::Devices::load(&ws.devices_path()).expect("devices")
}

fn keys(f: &Fixture) -> sapphire_framework::remote_server::KeyStore {
    sapphire_framework::remote_server::KeyStore::load(&f.keys).expect("keys")
}

#[test]
fn a_device_is_registered_and_gets_exactly_one_key() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");

    let devices = devices(&f);
    let device = devices.resolve("laptop").expect("device exists");
    assert!(!device.is_retired());

    let keys = keys(&f);
    let entries: Vec<_> = keys.entries().iter().filter(|k| k.device_id == Some(device.id)).collect();
    assert_eq!(entries.len(), 1, "a device gets one key, no more and no fewer");
}

#[test]
fn a_device_under_an_unknown_user_is_refused_and_mints_no_key() {
    let f = fixture();
    add_device(&f, "laptop", "nobody").expect_err("unknown user must be refused");
    assert!(
        keys(&f).entries().is_empty(),
        "a refused device must leave no key behind -- an unattributable key is \
         exactly what this design exists to prevent"
    );
}

#[test]
fn rotate_keeps_the_device_and_changes_the_token() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");

    let device_id = devices(&f).resolve("laptop").expect("device").id;
    let before = keys(&f)
        .entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
        .expect("key")
        .token
        .clone();

    run(&f, Command::Device(DeviceCommand::Rotate {
        selector: "laptop".into(),
        expires_in: None,
    }))
    .expect("rotate");

    let after_device = devices(&f).resolve("laptop").expect("device").id;
    let after = keys(&f)
        .entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
        .expect("key")
        .token
        .clone();

    assert_eq!(after_device, device_id, "rotation keeps the device's identity");
    assert_ne!(after, before, "rotation replaces the token");
}

#[test]
fn a_retired_device_keeps_resolving_to_its_name() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");
    let device_id = devices(&f).resolve("laptop").expect("device").id;

    run(&f, Command::Device(DeviceCommand::Retire { selector: "laptop".into() }))
        .expect("retire");

    let devices = devices(&f);
    let device = devices.get(device_id).expect(
        "a retired device is a tombstone, not a deletion -- a record that named \
         this id must still resolve to a name",
    );
    assert!(device.is_retired());
    assert_eq!(device.name, "laptop");

    assert!(
        keys(&f).authenticate(&"unused".to_string()).is_none(),
        "sanity: nothing authenticates against a bogus token"
    );
}

#[test]
fn parse_duration_accepts_the_documented_forms() {
    assert_eq!(identity::parse_duration("90d").unwrap().num_days(), 90);
    assert_eq!(identity::parse_duration("12h").unwrap().num_hours(), 12);
    identity::parse_duration("soon").expect_err("not a duration");
}
```

- [ ] **Step 2: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-server --test identity
```

Expected: FAIL — `identity` does not exist.

- [ ] **Step 3: Implement `identity.rs`**

Create `sapphire-ledger-server/src/identity.rs` implementing `run` and `parse_duration` against the signatures above. Requirements, each of which a test above pins:

- `User::Add` loads `users.toml` from `Workspace::users_path()`, calls `Users::add`, and prints the new user's id and name to **stderr** — this command has no machine-readable output, and reserving stdout keeps the contract uniform.
- `User::List` prints id, name and description.
- `Device::Add` resolves the user **first** and returns its error unchanged if unknown, before touching the key store. Then `Devices::add(name, description, Some(user.id))`, retrying up to 8 times while the error message contains `collides with an existing device`. Then `KeyStore::generate("sl", None, Some(device.id), Some(name.to_string()), expires_at)`. Print **only the raw token** to stdout, and everything else (the device id, the reminder that the token is not recoverable) to stderr.
- `Device::List` prints one row per device: id, name, user name resolved through `user_id`, `retired` when `is_retired()`, and the masked token — first four characters then `…`. A device with no key shows `-`.
- `Device::Rotate` resolves the device, finds the key whose `device_id` matches, and calls `KeyStore::rotate("sl", &key.id.to_string(), expires_at)`. Prints only the new token to stdout.
- `Device::Retire` calls `Devices::retire(selector)` and then `KeyStore::revoke` for the matching key. A device with no key still retires.
- `parse_duration` wraps `humantime::parse_duration` and converts to `chrono::Duration`; `expires_at` is `Utc::now() + duration`.

Write the key-store lookup by device once, as a private helper, and use it from `Rotate`, `Retire` and `List` — three copies of the same `find` is the kind of duplication a reviewer should reject.

- [ ] **Step 4: Wire it into `main.rs`**

Replace the `Some(_) => anyhow::bail!(...)` arm:

```rust
        Some(command) => identity::run(command, &ledger_dir, &keys_path),
```

and add `pub mod identity;` to `lib.rs`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-server
cargo clippy -p sapphire-ledger-server --all-targets -- -D warnings
```

- [ ] **Step 6: Verify the stdout contract by hand**

```bash
cargo run -p sapphire-ledger-server -- --ledger-dir <a test ledger> user add me
cargo run -p sapphire-ledger-server -- --ledger-dir <a test ledger> device add laptop --user me > token.txt
```

`token.txt` must contain exactly one line: the token. Any warning or log line in it is a bug in this task, not a nuisance.

- [ ] **Step 7: Commit**

```bash
git add sapphire-ledger-server/
git commit -m "feat(server): manage clients as devices belonging to users

A key names a device and a device names a user, so a record written through
this server can later say which of them wrote it. There is no way to mint a
key without a device: an unattributable key is the thing this prevents.

Retirement tombstones rather than deletes, so an id already written into a
record still resolves to a name."
```

---

### Task 4: Watch for duplicate ids

**Files:**
- Create: `sapphire-ledger-server/src/watch.rs`
- Modify: `sapphire-ledger-server/src/lib.rs`, `src/serve.rs`
- Test: `sapphire-ledger-server/tests/watch.rs`

**Interfaces:**
- Consumes: `sapphire_ledger_core::{load_workspace, ValidationIssue}`.
- Produces: `watch::duplicate_report(root: &Path) -> anyhow::Result<Vec<String>>` — one human-readable line per duplicate, each naming the id **and every path carrying it**; and `watch::spawn(root: PathBuf) -> tokio::task::JoinHandle<()>`.

`ValidationIssue` carries ids but **not paths**, and the paths are what a human needs in order to fix this. So this module re-walks the record directories itself to map id → paths rather than reformatting `validate()`'s output.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-server/tests/watch.rs`:

```rust
use sapphire_ledger_server::watch;

/// Two transactions with one id, in different months -- the shape the sync
/// model actually produces, since the path embeds the record's own date.
#[test]
fn a_duplicate_transaction_id_is_reported_with_every_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    for month in ["05", "06"] {
        let path = root.join(format!("transactions/2026/{month}/dup0001.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            r#"
id = "dup0001"
date = "2026-05-21"
narration = "same id, two months"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Food"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Cash"
amount = "-10"
currency = "JPY"
"#,
        )
        .expect("write");
    }

    let report = watch::duplicate_report(root).expect("report");
    assert_eq!(report.len(), 1, "one duplicated id, one line: {report:?}");
    let line = &report[0];
    assert!(line.contains("dup0001"), "must name the id: {line}");
    assert!(line.contains("2026/05"), "must name every path: {line}");
    assert!(line.contains("2026/06"), "must name every path: {line}");
}

#[test]
fn a_clean_ledger_reports_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");
    assert!(watch::duplicate_report(dir.path()).expect("report").is_empty());
}

#[test]
fn a_duplicate_account_name_is_reported_too() {
    // Two account files at different paths naming themselves the same thing:
    // what a raced rename leaves behind.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    for leaf in ["Bar", "Foo"] {
        let path = root.join(format!("accounts/Assets/{leaf}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            format!(
                r#"
id = "acct{leaf}"
name = "Assets:Foo"
type = "Asset"
opened_at = "2026-01-01"
"#
            ),
        )
        .expect("write");
    }

    let report = watch::duplicate_report(root).expect("report");
    assert!(
        report.iter().any(|l| l.contains("Assets:Foo")),
        "a duplicated account name must be reported: {report:?}"
    );
}
```

- [ ] **Step 2: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-server --test watch
```

Expected: FAIL — `watch` does not exist.

- [ ] **Step 3: Implement `watch.rs`**

Create `sapphire-ledger-server/src/watch.rs`. Requirements:

- `duplicate_report` walks `transactions/`, `assertions/`, `prices/` and `accounts/` with `sapphire_ledger_core::walk_toml_files`, loads each record, and builds `id -> Vec<PathBuf>` per kind plus `account name -> Vec<PathBuf>`. Any entry with more than one path becomes a line naming the kind, the id (or name), and every path, relative to `root`.
- A file that fails to parse is **skipped, not fatal** — this runs on a timer against a directory other processes are writing to, and one malformed record must not silence the duplicate report. Log it at `debug`.
- `spawn(root)` runs `duplicate_report` immediately and then every `CHECK_INTERVAL`, logging each line at `WARN`. Define `const CHECK_INTERVAL: Duration = Duration::from_secs(300);` with a comment saying it is a constant on purpose — nothing yet suggests a different number, and a knob nobody asked for is a knob to maintain.
- Nothing is ever modified. Add a module doc comment saying so and why: the journal converges duplicates automatically by re-iding one and keeping both, which is right for notes and wrong for money, and last-writer-wins on a client-supplied clock could delete a correct transaction. Detection is the deliverable; resolution is a decision to make with a real duplicate in front of you.

- [ ] **Step 4: Start the watch from `serve::run`**

In `serve.rs`, after the listener binds and before `axum::serve`:

```rust
    let watch = crate::watch::spawn(ledger_dir.to_path_buf());
```

and abort it after serving returns:

```rust
    watch.abort();
```

Add `pub mod watch;` to `lib.rs`.

- [ ] **Step 5: Run the tests and commit**

```bash
cargo test -p sapphire-ledger-server
cargo clippy -p sapphire-ledger-server --all-targets -- -D warnings
git add sapphire-ledger-server/
git commit -m "feat(server): report duplicate ids, and never resolve them

The sync model resolves per path, last-writer-wins on a client-supplied
clock, and a ledger record's path embeds mutable data -- a transaction's date,
an account's name. So a client that was offline across a rename can resurrect
the old path and leave one id at two paths.

This reports it and stops. Converging automatically the way the journal does
would double-count a transaction, and picking a winner by timestamp would let
a skewed clock delete a correct one."
```

---

### Task 5: Documentation

**Files:**
- Modify: `README.md`, `docs/design.md`

- [ ] **Step 1: Update the project structure and status**

In `README.md`, change the `sapphire-ledger-server/` line in the project-structure tree from `(planned)` to a description of what it does, and update the Status section: the MCP server is now reachable over HTTP with per-device authentication; `/rpc` sync is still absent, so a correction still has to be made on the machine holding the files.

- [ ] **Step 2: Document the server in `docs/design.md`**

In the "MCP server" section, add a subsection covering: the server crate and what it composes; that `/rpc` is deliberately not mounted and why; the device/user model and the two-file split (registry in the workspace, keys host-local); the `Host` allowlist and the refuse-to-start rule; and that duplicate ids are reported but never resolved. Link the spec for the full reasoning rather than repeating it.

Add the CLI to the same section:

```
sapphire-ledger-server --ledger-dir DIR              # serve
sapphire-ledger-server user add <name>
sapphire-ledger-server user list
sapphire-ledger-server device add <name> --user <selector>
sapphire-ledger-server device list
sapphire-ledger-server device rotate <selector>
sapphire-ledger-server device retire <selector>
```

- [ ] **Step 3: Update both Status lists**

`sapphire-ledger-server` moves from 🚧 to ✅ in `docs/design.md`, with a new 🚧 line for `/rpc` sync so the absence stays visible rather than reading as delivered.

- [ ] **Step 4: Verify and commit**

```bash
grep -rn "planned" README.md docs/design.md
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: remaining "planned" mentions are `/rpc` and later phases only.

```bash
git add README.md docs/design.md
git commit -m "docs: describe the server that now exists"
```

---

## Self-Review

**Spec coverage.** Every section of the spec maps to a task. "What we compose" and the crate layout → Tasks 1-2. "The `Host` allowlist is an argument" → Task 1, with the quiet-failure case tested explicitly. "Identity" → Task 3, including the two-file split (Task 2's `default_keys_path` doc comment states it, Task 3's fixture exercises it). "`last_updated_by`" → correctly **not** implemented; Task 3 asserts a key's `device_id` is set, which is the hook it will use. "Exposure" and the refuse-to-start divergence → Task 2. "Duplicate ids" → Task 4. Testing → distributed across the tasks that own each behaviour. Risks → nothing to implement.

**One spec claim adjusted.** The spec's testing list says "an authenticated request's `Authenticated.device_id` is the device that owns the key". Reading it through an actual request means adding a route purely to observe it, since `/mcp` is rmcp's service and not ours to instrument — inventing an endpoint for a test is worse than the gap it closes. Task 3 asserts the same wiring one level down: the key the CLI mints carries the device's id. The spec should be corrected to say that; it is the same guarantee, verified where we own the code.

**Placeholder scan.** No TBD/TODO. Task 3's Step 3 and Task 4's Step 3 give requirements rather than a full listing — deliberately, because both are mechanical assembly over signatures quoted verbatim above them, and every behaviour is pinned by a test written out in full. Task 5 is prose whose content is described rather than dictated, which is right for docs.

**Type consistency.** `mcp_router`'s four parameters are identical in Task 1's definition and Task 2's call. `serve::{default_keys_path, build_state, check_exposure, run}` match between Task 2's interface block, its implementation and its tests. `identity::run(command, ledger_dir, keys_path)` matches between Task 3's interface block, the test's `run` helper and `main.rs`'s call arm. `watch::{duplicate_report, spawn}` match between Task 4's interface block, its tests and `serve.rs`'s use. `Command`/`UserCommand`/`DeviceCommand` variants are defined once in Task 2's `cli.rs` and constructed by name in Task 3's tests.

**Known risks.** `sapphire_ledger_core::LEDGER_CTX.workspace_at(...)` is used in Task 3's test fixture to reach `devices_path()`; confirm that method's exact name against the framework's `AppContext` before writing it, and use whatever the crate actually exposes for "open the workspace rooted here". `humantime::parse_duration` returns `std::time::Duration`, so the conversion to `chrono::Duration` needs `chrono::Duration::from_std`, which is fallible for very large values. Both are the kind of thing the first compile settles.
