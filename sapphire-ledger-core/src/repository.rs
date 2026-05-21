use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::account::Account;
use crate::assertion::Assertion;
use crate::config::Config;
use crate::error::Result;
use crate::transaction::Transaction;
use crate::workspace::{
    ACCOUNTS_DIR, ASSERTIONS_DIR, CONFIG_FILE, TOML_EXTENSION, TRANSACTIONS_DIR, WORKSPACE_DIR,
};

/// Read a TOML file and deserialize it into `T`.
pub fn load_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

/// Serialize `value` as TOML and write it to `path`, creating parent
/// directories as needed.
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(value)?;
    fs::write(path, text)?;
    Ok(())
}

/// Recursively collect all `.toml` files under `root`. Returns an empty Vec
/// if `root` does not exist (the directory is allowed to be absent for an
/// empty workspace).
pub fn walk_toml_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !root.exists() {
        return Ok(result);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file()
                && path.extension().is_some_and(|ext| ext == TOML_EXTENSION)
            {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(result)
}

/// An eagerly-loaded in-memory view of a sapphire-ledger workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: Config,
    pub accounts: Vec<Account>,
    pub transactions: Vec<Transaction>,
    pub assertions: Vec<Assertion>,
}

/// Load all records from a workspace at `root` (the directory containing
/// `.sapphire-ledger/`).
pub fn load_workspace(root: &Path) -> Result<Workspace> {
    let config: Config = load_toml(&root.join(WORKSPACE_DIR).join(CONFIG_FILE))?;

    let accounts = walk_toml_files(&root.join(ACCOUNTS_DIR))?
        .iter()
        .map(|p| load_toml::<Account>(p))
        .collect::<Result<Vec<_>>>()?;

    let transactions = walk_toml_files(&root.join(TRANSACTIONS_DIR))?
        .iter()
        .map(|p| load_toml::<Transaction>(p))
        .collect::<Result<Vec<_>>>()?;

    let assertions = walk_toml_files(&root.join(ASSERTIONS_DIR))?
        .iter()
        .map(|p| load_toml::<Assertion>(p))
        .collect::<Result<Vec<_>>>()?;

    Ok(Workspace {
        root: root.to_path_buf(),
        config,
        accounts,
        transactions,
        assertions,
    })
}
