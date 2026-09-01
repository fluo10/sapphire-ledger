//! Core data model and storage for sapphire-ledger.
//!
//! Defines the on-disk TOML record types (Account, Transaction, Assertion,
//! workspace Config) and their validation. SQLite cache and higher-level
//! repository operations will be added in later phases.

pub mod account;
pub mod assertion;
pub mod config;
pub mod error;
pub mod ops;
pub mod prices;
pub mod repository;
pub mod transaction;
pub mod validate;
pub mod workspace;

pub use account::{Account, AccountType, account_name_segments, describe_ref, resolve_account};
pub use assertion::{Assertion, Balance};
pub use config::{CURRENT_SCHEMA_VERSION, CacheConfig, Config};
pub use error::{Error, Result};
pub use prices::{Price, PriceEntry};
pub use repository::{Workspace, load_toml, load_workspace, save_toml, walk_toml_files};
pub use transaction::{Posting, Transaction, TransactionStatus};
pub use validate::ValidationIssue;
pub use workspace::{
    PRICES_DIR, account_name_from_relative_path, account_relative_path, assertion_relative_path,
    find_workspace_root, init_workspace, price_relative_path, transaction_relative_path,
};
