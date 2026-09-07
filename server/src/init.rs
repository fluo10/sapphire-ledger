//! Creating a ledger workspace for this server to serve.
//!
//! The same job `sapphire-ledger init` does, offered here so that setting up a
//! server does not require a second binary. It creates the workspace and stops:
//! the registry and the first device's token come from `user add` and
//! `device add`, which already exist and already own that concern.

use std::path::Path;

use anyhow::Context as _;

/// Create a ledger workspace at `root`.
///
/// Refuses a directory that is already a workspace — that check lives in
/// `sapphire_ledger_core::init_workspace`, and the error it returns says so.
///
/// Everything printed here goes to **stderr**. `stdout` in this binary carries
/// the raw token from `device add` and `device rotate` and nothing else, and a
/// command that starts a setup whose next step prints a secret should not be
/// the one that muddies the stream.
pub fn run(root: &Path, base_currency: &str) -> anyhow::Result<()> {
    sapphire_ledger_core::init_workspace(root, base_currency)
        .with_context(|| format!("failed to initialize a ledger at {}", root.display()))?;

    eprintln!(
        "Initialized a sapphire-ledger workspace at {} (base currency: {base_currency})",
        root.display()
    );
    eprintln!();
    eprintln!("Next, register who the clients belong to and mint one a token:");
    eprintln!(
        "  sapphire-ledger-server --ledger-dir {} user add <your name>",
        root.display()
    );
    eprintln!(
        "  sapphire-ledger-server --ledger-dir {} device add <device name> --user <your name>",
        root.display()
    );
    eprintln!();
    eprintln!("`device add` prints the token to stdout, once. It is not recoverable afterwards.");

    Ok(())
}
