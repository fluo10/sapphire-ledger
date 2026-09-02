//! MCP server logic for sapphire-ledger.
//!
//! Consumed by `sapphire-ledger-cli` (stdio transport) and, later, by
//! `sapphire-ledger-server` (HTTP transport).

pub mod server;

pub use server::{SapphireLedgerServer, run};
