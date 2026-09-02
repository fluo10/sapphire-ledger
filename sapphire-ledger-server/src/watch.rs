//! Watches the workspace for one id claimed by more than one file, and does
//! nothing else about it.
//!
//! A ledger record's path embeds mutable data: a transaction's or
//! assertion's path is derived from its *date*, an account's from its
//! *name*. The sync model that will land in `/rpc` resolves conflicts per
//! path, last-writer-wins, on a client-supplied wall clock, with deletes as
//! tombstones under the same comparison. A client that was offline across a
//! rename or a date correction can push to the old path with a later
//! timestamp; the tombstone loses, and the record now exists at two paths
//! under one id.
//!
//! This module reports that and stops. It never resolves anything, and that
//! is a decision, not an omission:
//!
//! - The sibling `sapphire-journal` converges automatically by re-iding the
//!   later arrival and keeping both records. That is right for notes -- a
//!   note that exists twice is at worst annoying. For a ledger it would
//!   double-count a transaction, which is a wrong balance.
//! - The obvious alternative -- last-writer-wins on the record's own
//!   `updated_at` -- is worse, not better: that timestamp is client-supplied
//!   wall clock, so a machine with a skewed clock could silently delete a
//!   correct transaction.
//!
//! Where money is concerned, a loud broken state beats a quiet resolved one.
//! Automatic convergence is a decision to make with a real duplicate in
//! front of you, not one to bake in ahead of time.
//!
//! Nothing here ever writes, moves or deletes a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sapphire_ledger_core::{Account, Assertion, PriceEntry, Transaction};

/// How often the watch re-walks the workspace looking for duplicate ids.
///
/// A constant, not a setting: nothing about this task suggests a workspace
/// needs a different cadence, and a knob nobody asked for is a knob someone
/// else has to maintain.
const CHECK_INTERVAL: Duration = Duration::from_secs(300);

/// Walk the record directories under `root` and report every id (or, for
/// accounts, every name) claimed by more than one file.
///
/// Returns one human-readable line per duplicate, naming the id and every
/// path carrying it. `ValidationIssue` -- what `Workspace::validate()`
/// already returns, and which already flags duplicate ids -- carries the ids
/// but not the paths, and the paths are what a human needs to fix this. So
/// this walks the directories itself with `walk_toml_files` rather than
/// reformatting `validate()`'s output.
///
/// A file that fails to parse is skipped, not fatal: this runs on a timer
/// against a directory other processes are actively writing to, and one
/// malformed record must not silence the duplicate report for everything
/// else. It is logged at `debug`.
pub fn duplicate_report(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    lines.extend(duplicates_of::<Transaction, _>(
        root,
        "transactions",
        "transaction id",
        |t| t.id.clone(),
    )?);
    lines.extend(duplicates_of::<Assertion, _>(
        root,
        "assertions",
        "assertion id",
        |a| a.id.clone(),
    )?);
    lines.extend(duplicates_of::<PriceEntry, _>(
        root,
        "prices",
        "price id",
        |p| p.id.clone(),
    )?);
    // Accounts are keyed by name, not id: the racy case is a rename, and a
    // name-only posting resolves by whichever account happens to own that
    // name -- so two accounts sharing a name is the broken state, even if
    // their ids differ.
    lines.extend(duplicates_of::<Account, _>(
        root,
        "accounts",
        "account name",
        |a| a.name.clone(),
    )?);
    lines.sort();
    Ok(lines)
}

/// Load every `.toml` record of type `T` under `root/dir`, group by the key
/// `key_of` extracts, and return one report line for every key with more
/// than one path.
fn duplicates_of<T, F>(root: &Path, dir: &str, kind: &str, key_of: F) -> anyhow::Result<Vec<String>>
where
    T: serde::de::DeserializeOwned,
    F: Fn(&T) -> String,
{
    let mut by_key: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for path in sapphire_ledger_core::walk_toml_files(&root.join(dir))? {
        match sapphire_ledger_core::load_toml::<T>(&path) {
            Ok(record) => by_key.entry(key_of(&record)).or_default().push(path),
            Err(err) => {
                let err = anyhow::Error::from(err);
                tracing::debug!(
                    path = %path.display(),
                    "skipping a record that failed to parse: {err:#}"
                );
            }
        }
    }

    let mut lines: Vec<String> = by_key
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(key, mut paths)| {
            paths.sort();
            let rendered: Vec<String> = paths
                .iter()
                .map(|path| display_relative(root, path))
                .collect();
            format!("duplicate {kind} {key}: {}", rendered.join(", "))
        })
        .collect();
    lines.sort();
    Ok(lines)
}

/// Render `path` relative to `root` with `/` separators regardless of
/// platform, so a report line reads the same on the machine that generated
/// it as it does wherever it gets pasted.
fn display_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Spawn the background watch: run [`duplicate_report`] immediately, then
/// again every [`CHECK_INTERVAL`], logging each line it produces at `WARN`.
///
/// Only duplicates are logged here, and only at `WARN` -- the rest of
/// `Workspace::validate()`'s output is the ledger's ordinary business and
/// belongs to the `validate_workspace` MCP tool, not to the server's log.
pub fn spawn(root: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        loop {
            interval.tick().await;
            match duplicate_report(&root) {
                Ok(lines) => {
                    for line in lines {
                        tracing::warn!("{line}");
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        root = %root.display(),
                        "duplicate-id watch could not walk the workspace: {err:#}"
                    );
                }
            }
        }
    })
}
