//! In-memory session state: an open ledger workspace.
//!
//! [`LedgerState`] is the single object frontends (CLI, MCP, GUI) hold while a
//! ledger is active, mirroring `JournalState` in sapphire-journal.
//!
//! It currently holds an eagerly-loaded [`Workspace`] and nothing else. The
//! search and mtime-tracking infrastructure the framework offers has no
//! consumer yet: no tool in this phase searches, and the ledger-specific index
//! is deliberately deferred. This type exists now so that adding them later is
//! a change inside one struct rather than a change to every caller.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::repository::{Workspace, load_workspace};
use crate::workspace::find_workspace_root;

/// An open ledger workspace.
pub struct LedgerState {
    root: PathBuf,
    workspace: Workspace,
}

impl LedgerState {
    /// Open the ledger rooted at `root` — the directory containing
    /// `.sapphire-ledger/` — and load every record.
    pub fn open(root: &Path) -> Result<Self> {
        let workspace = load_workspace(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            workspace,
        })
    }

    /// Walk upward from `start` to find a workspace, then open it.
    pub fn find(start: &Path) -> Result<Self> {
        let root = find_workspace_root(start)?;
        Self::open(&root)
    }

    /// The loaded records: a snapshot taken at `open` or the last `reload`.
    /// A write through `ops` does not update it.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Re-read every record from disk. Callers do this after writing, and
    /// periodically to pick up edits from git, sync, or a human editor.
    pub fn reload(&mut self) -> Result<()> {
        self.workspace = load_workspace(&self.root)?;
        Ok(())
    }
}
