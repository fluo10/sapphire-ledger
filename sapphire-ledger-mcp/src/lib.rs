//! MCP server logic for sapphire-ledger.
//!
//! Consumed by `sapphire-ledger-cli` (stdio transport) and, later, by
//! `sapphire-ledger-server` (HTTP transport).

pub mod server;

#[cfg(feature = "http-server")]
pub mod http;

pub use server::{SapphireLedgerServer, run};

#[cfg(feature = "http-server")]
pub use http::mcp_router;
