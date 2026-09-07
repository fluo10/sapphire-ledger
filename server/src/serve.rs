//! Building and running the HTTP server.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use sapphire_framework::remote_server::{KeyStore, ServerState, protect};
use tokio_util::sync::CancellationToken;

/// Where the key file goes when `--keys` is not given: this app's cache
/// directory, **not** the workspace — and within it, the subdirectory
/// belonging to *this* ledger rather than the cache root.
///
/// Two axes, and both matter:
///
/// **Cache, not workspace.** The workspace is what syncs once `/rpc` exists,
/// and a token that syncs is a token on every machine that ever pulled. The
/// device and user tables are the opposite case and do live in the workspace
/// — they are content, and a device id is only resolvable elsewhere if they
/// travel.
///
/// **Per workspace, not per host.** `cache_dir_for` gives
/// `{cache}/{path_uuid}/`, so a personal ledger and a business ledger on one
/// machine get separate key files. Sharing one would mean a device
/// registered under the first authenticates in full against the second,
/// because `protect()` checks that a bearer names *a* key — it never
/// cross-checks that key's `device_id` against the served workspace's
/// `devices.toml`. That would also break `last_updated_by` before it is
/// written: the field holds a `device_id` and its resolution is supposed to
/// complete inside this application, which it cannot if a record carries an
/// id no local `devices.toml` knows. `sapphire-journal-server` splits the
/// same way, for the same reason.
///
/// `LEDGER_CTX` panics if read before `sapphire_ledger_core::init_app_context`
/// has run — this function assumes the caller already did that (`main` does,
/// as its first statement). A path helper that quietly initialised global
/// state on the side would be a surprise to its next caller and would leave
/// `LEDGER_CTX`'s data directory still unset for whoever reads that instead.
pub fn default_keys_path(ledger_dir: &Path) -> anyhow::Result<PathBuf> {
    let cache = sapphire_ledger_core::LEDGER_CTX.cache_dir_for(ledger_dir);
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("failed to create {}", cache.display()))?;
    Ok(cache.join("keys.toml"))
}

/// Build the server state. Only the key store is configured: no resolver and
/// no workspace store, because `/rpc` is not mounted.
///
/// `data_dir` is where the framework would keep per-workspace sync state. It
/// is required by the constructor and unused here, so it is set to the key
/// file's own directory -- not necessarily the cache directory: with
/// `--keys` pointing elsewhere, it isn't. Harmless while nothing writes it.
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

/// Build the whole router this process serves: `/mcp` from
/// `sapphire-ledger-mcp`, wrapped in the framework's `protect`.
///
/// Split out of [`run`] so the composition is testable. [`run`] binds a
/// socket, which a test cannot drive; everything above the socket -- that
/// `protect` is actually applied, that a valid bearer gets through, that
/// `/rpc` is not mounted -- is the part worth pinning, and a `Router` can be
/// called directly with `tower::ServiceExt::oneshot`.
///
/// **`/rpc` is deliberately absent.** The framework's `router()` is never
/// merged here: `remote-server` is taken for `KeyStore` and `protect`, not
/// for sync, and sync has no client yet. Mounting it later is a `.merge()`
/// against this same `ServerState`.
pub fn build_router(
    state: Arc<ServerState>,
    ledger_dir: &Path,
    allowed_hosts: &[String],
    cancel: CancellationToken,
) -> anyhow::Result<axum::Router> {
    let ledger_state = sapphire_ledger_mcp::server::prepare_state(Some(ledger_dir), false)?;
    let shared = Arc::new(std::sync::Mutex::new(ledger_state));
    let mcp = sapphire_ledger_mcp::http::mcp_router(shared, cancel, None, allowed_hosts);
    Ok(protect(state, mcp))
}

/// Serve until Ctrl-C or SIGTERM, then drain.
pub async fn run(
    addr: SocketAddr,
    ledger_dir: &Path,
    state: Arc<ServerState>,
    allowed_hosts: &[String],
) -> anyhow::Result<()> {
    check_exposure(addr, allowed_hosts)?;

    // `build_state` always attaches a key store, so `protect` installs its
    // authenticating layer rather than its refusing one -- with zero usable
    // keys, every request just 401s from here on, silently. Unlike the wide
    // bind `check_exposure` refuses above, an empty key store is a
    // legitimate transient state while someone is setting up, so this only
    // warns instead of refusing to start.
    if !state.keys().is_some_and(|keys| keys.has_usable_key()) {
        tracing::warn!(
            "no usable API key configured -- every request will be refused until \
             the key file names at least one unexpired key"
        );
    }

    let cancel = CancellationToken::new();
    let app = build_router(
        Arc::clone(&state),
        ledger_dir,
        allowed_hosts,
        cancel.clone(),
    )?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!("sapphire-ledger-server listening on http://{addr}/mcp");

    let watch = crate::watch::spawn(ledger_dir.to_path_buf());

    let shutdown = cancel.clone();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown.cancel();
        })
        .await;

    watch.abort();

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
