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
///
/// `LEDGER_CTX` panics if read before `sapphire_ledger_core::init_app_context`
/// has run — this function assumes the caller already did that (`main` does,
/// as its first statement). A path helper that quietly initialised global
/// state on the side would be a surprise to its next caller and would leave
/// `LEDGER_CTX`'s data directory still unset for whoever reads that instead.
pub fn default_keys_path(_ledger_dir: &Path) -> anyhow::Result<PathBuf> {
    let cache = sapphire_ledger_core::LEDGER_CTX.cache_dir();
    std::fs::create_dir_all(cache)
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
