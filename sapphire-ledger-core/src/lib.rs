//! Core data model and storage for sapphire-ledger.
//!
//! Defines the on-disk TOML record types (Account, Transaction, Assertion,
//! workspace Config) and their validation. SQLite cache and higher-level
//! repository operations will be added in later phases.

pub mod account;
pub mod assertion;
pub mod config;
pub mod error;
pub mod transaction;
pub mod workspace;

pub use account::{Account, AccountType};
pub use assertion::{Assertion, Balance};
pub use config::{CURRENT_SCHEMA_VERSION, CacheConfig, Config};
pub use error::{Error, Result};
pub use transaction::{Posting, Price, Transaction, TransactionStatus};
pub use workspace::find_workspace_root;
