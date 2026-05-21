use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub const WORKSPACE_DIR: &str = ".sapphire-ledger";
pub const CONFIG_FILE: &str = "config.toml";
pub const CACHE_FILE: &str = "cache.sqlite";

pub const TRANSACTIONS_DIR: &str = "transactions";
pub const ACCOUNTS_DIR: &str = "accounts";
pub const ASSERTIONS_DIR: &str = "assertions";

/// Walk upward from `start` looking for a directory containing `.sapphire-ledger/`.
/// Returns the workspace root (the directory containing `.sapphire-ledger/`, not
/// `.sapphire-ledger/` itself).
pub fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let start_canonical = start.canonicalize()?;
    let mut current: &Path = &start_canonical;
    loop {
        if current.join(WORKSPACE_DIR).is_dir() {
            return Ok(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Err(Error::NotAWorkspace(start.to_path_buf())),
        }
    }
}
