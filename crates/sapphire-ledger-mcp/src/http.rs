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
    let service =
        StreamableHttpService::new(factory, Arc::new(LocalSessionManager::default()), config);

    axum::Router::new().route_service("/mcp", service)
}
