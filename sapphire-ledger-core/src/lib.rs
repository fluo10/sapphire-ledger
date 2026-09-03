//! Core data model and storage for sapphire-ledger.
//!
//! Defines the on-disk TOML record types (Account, Transaction, Assertion,
//! PriceEntry, workspace Config), their validation, and the [`ops`] write
//! path that mints ids, resolves account references and enforces the
//! refuse-to-overwrite rule. There is no SQLite cache here and none is
//! planned — see "Cache strategy" in `docs/design.md`.

pub mod account;
pub mod assertion;
pub mod config;
pub mod error;
pub mod ops;
pub mod prices;
pub mod repository;
pub mod state;
pub mod transaction;
pub mod validate;
pub mod workspace;

pub use account::{Account, AccountType, account_name_segments, describe_ref, resolve_account};
pub use assertion::{Assertion, Balance};
pub use config::{CURRENT_SCHEMA_VERSION, CacheConfig, Config};
pub use error::{Error, Result};
pub use prices::{Price, PriceEntry};
pub use repository::{Workspace, load_toml, load_workspace, save_toml, walk_toml_files};
pub use state::LedgerState;
pub use transaction::{Posting, Transaction, TransactionStatus};
pub use validate::ValidationIssue;
pub use workspace::{
    PRICES_DIR, account_name_from_relative_path, account_relative_path, assertion_relative_path,
    find_workspace_root, init_workspace, price_relative_path, transaction_relative_path,
};

/// Process-wide application context, naming the cache and data directories
/// the framework uses. Mirrors `JOURNAL_CTX` in sapphire-journal.
pub static LEDGER_CTX: sapphire_workspace::AppContext =
    sapphire_workspace::AppContext::new("sapphire-ledger");

/// Point [`LEDGER_CTX`] at this machine's cache and data directories.
///
/// `AppContext::cache_dir()` panics if this has not run, so every binary that
/// touches the context calls this first. Mirrors `sapphire-journal-core`'s
/// `init_app_context`. Idempotent: `AppContext`'s setters are first-writer-wins,
/// so calling it twice is harmless and calling it from a test is fine.
pub fn init_app_context() {
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir().join(".cache"))
        .join("sapphire-ledger");
    let data = dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join(".local").join("share"))
        .join("sapphire-ledger");
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&data);
    LEDGER_CTX.set_cache_dir(cache);
    LEDGER_CTX.set_data_dir(data);
}
