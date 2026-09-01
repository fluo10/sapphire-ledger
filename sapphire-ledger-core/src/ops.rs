//! The single write path for every record kind.
//!
//! CLI, MCP, GUI and importers all funnel through here so that id
//! generation, path resolution, validation and the refuse-to-overwrite rule
//! exist in exactly one place.

use grain_id::GrainId;

/// Mint a time-ordered record id, for records whose id is their filename.
///
/// `GrainId::now_unix()` has decisecond resolution, so records made in the
/// same tenth of a second collide; the create functions re-mint rather than
/// overwrite. Time ordering is what makes a `{year}/{MM}/` listing readable.
pub fn new_id() -> String {
    GrainId::now_unix().to_string()
}

/// Mint a random record id, for records whose id is *not* their filename.
///
/// Accounts use this. Their id does no ordering work — `opened_at` carries
/// the meaningful date — and a chart of accounts is typically created in one
/// sitting, where time-ordered ids would all share a long leading prefix
/// exactly when there are the most to tell apart at a CLI prompt.
pub fn new_random_id() -> String {
    GrainId::random().to_string()
}

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::account::{Account, AccountType, resolve_account};
use crate::assertion::{Assertion, Balance};
use crate::error::{Error, Result};
use crate::prices::PriceEntry;
use crate::repository::{load_toml, save_toml, walk_toml_files};
use crate::transaction::{Posting, Transaction, TransactionStatus};
use crate::workspace::{
    ACCOUNTS_DIR, ASSERTIONS_DIR, PRICES_DIR, TRANSACTIONS_DIR, account_relative_path,
    assertion_relative_path, price_relative_path, transaction_relative_path,
};

/// How many times to re-mint an id when the destination is already taken.
///
/// `new_id()` is deterministic within a decisecond, so re-calling it on a
/// collision would mint the same id forever; [`mint_free_id`] falls back to
/// `new_random_id()` after the first attempt to actually break the tie. An
/// account's random id colliding with an existing one is astronomically
/// unlikely, but the retry must still terminate.
const ID_ATTEMPTS: usize = 32;

fn now() -> DateTime<FixedOffset> {
    Local::now().into()
}

fn refuse_existing(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::Validation(format!(
            "{} already exists; refusing to overwrite",
            path.display()
        )));
    }
    Ok(())
}

/// Load just the accounts. Creates need them to resolve references and to
/// check id uniqueness, and at household scale re-walking is free.
fn load_accounts(root: &Path) -> Result<Vec<Account>> {
    walk_toml_files(&root.join(ACCOUNTS_DIR))?
        .iter()
        .map(|p| load_toml::<Account>(p))
        .collect()
}

/// Mint an id that no record of this kind is already using.
///
/// Uniqueness is scoped to the **record kind**, not to one `{year}/{MM}/`
/// directory. The destination path embeds the record's own date, so checking
/// only whether `dest` exists would let two records minted inside the same
/// decisecond with dates in different months take the same id — their paths
/// differ, so neither collision check would fire. `kind_dir` is walked to
/// collect every id already taken across the kind.
///
/// Time-ordering is an ergonomic nicety on top of uniqueness, so the first
/// attempt uses the time-ordered generator, which is what a record written on
/// its own gets. `new_id()` has decisecond resolution and no random component,
/// so it is deterministic within that window: a collision means another record
/// of this kind was minted in this same decisecond, and re-calling it would
/// mint the identical id forever. Only a different generator can break that
/// tie, so every attempt after the first is random.
fn mint_free_id<F>(root: &Path, kind_dir: &str, relative: F) -> Result<(String, PathBuf)>
where
    F: Fn(&str) -> PathBuf,
{
    let taken: HashSet<String> = walk_toml_files(&root.join(kind_dir))?
        .iter()
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();

    for attempt in 0..ID_ATTEMPTS {
        let id = if attempt == 0 {
            new_id()
        } else {
            new_random_id()
        };
        if taken.contains(&id) {
            continue;
        }
        let dest = root.join(relative(&id));
        // The walk above is a snapshot; still refuse an existing destination.
        if !dest.exists() {
            return Ok((id, dest));
        }
    }
    Err(Error::Validation(format!(
        "could not mint a free record id after {ID_ATTEMPTS} attempts"
    )))
}

/// Rewrite a set of postings so each one carries the resolved `account_id`
/// and the account's current `account_name`.
///
/// A record must never reach disk carrying only a name: the name is the part
/// allowed to go stale.
fn resolve_postings(postings: Vec<Posting>, accounts: &[Account]) -> Result<Vec<Posting>> {
    let by_id: HashMap<&str, &Account> = accounts.iter().map(|a| (a.id.as_str(), a)).collect();
    let by_name: HashMap<&str, &Account> = accounts.iter().map(|a| (a.name.as_str(), a)).collect();

    postings
        .into_iter()
        .map(|p| {
            let account = resolve_account(
                p.account_id.as_deref(),
                p.account_name.as_deref(),
                &by_id,
                &by_name,
            )?;
            if !account.allows_currency(&p.currency) {
                return Err(Error::Validation(format!(
                    "posts {} to {}, but that account only allows {}",
                    p.currency,
                    account.name,
                    account.currencies.join(", "),
                )));
            }
            Ok(Posting {
                account_id: Some(account.id.clone()),
                account_name: Some(account.name.clone()),
                ..p
            })
        })
        .collect()
}

/// Write a new account.
///
/// A duplicate name is a hard error. The path comes from the name, so the
/// refuse-to-overwrite check catches that on its own — but only while every
/// account file sits at the path its name implies, and nothing enforces that:
/// a hand edit, a half-applied rename or a sync artifact can leave an account
/// named `Assets:Foo` in `accounts/Assets/Bar.toml`. So the name is also
/// checked against the accounts already on disk.
///
/// The id is random and is *not* in the path, so the refuse-to-overwrite
/// check cannot see an id collision either — that is checked here too.
pub fn create_account(
    root: &Path,
    name: String,
    account_type: AccountType,
    currencies: Vec<String>,
    opened_at: NaiveDate,
    description: Option<String>,
) -> Result<(String, PathBuf)> {
    let dest = root.join(account_relative_path(&name)?);
    refuse_existing(&dest)?;

    let existing = load_accounts(root)?;
    if existing.iter().any(|a| a.name == name) {
        return Err(Error::Validation(format!(
            "an account named {name} already exists"
        )));
    }
    let taken: std::collections::HashSet<&str> = existing.iter().map(|a| a.id.as_str()).collect();

    let id = (0..ID_ATTEMPTS)
        .map(|_| new_random_id())
        .find(|candidate| !taken.contains(candidate.as_str()))
        .ok_or_else(|| {
            Error::Validation(format!(
                "could not mint a free account id after {ID_ATTEMPTS} attempts"
            ))
        })?;

    let account = Account {
        id: id.clone(),
        name,
        account_type,
        currencies,
        opened_at,
        closed_at: None,
        description,
    };
    save_toml(&dest, &account)?;
    Ok((id, dest))
}

/// Write a new transaction, resolving and validating before touching disk.
pub fn create_transaction(
    root: &Path,
    date: NaiveDate,
    narration: String,
    payee: Option<String>,
    tags: Vec<String>,
    status: Option<TransactionStatus>,
    postings: Vec<Posting>,
) -> Result<(String, PathBuf)> {
    let accounts = load_accounts(root)?;
    let postings = resolve_postings(postings, &accounts)?;

    let (id, dest) = mint_free_id(root, TRANSACTIONS_DIR, |id| {
        transaction_relative_path(date, id)
    })?;
    let timestamp = now();
    let transaction = Transaction {
        id: id.clone(),
        date,
        narration,
        payee,
        tags,
        status,
        created_at: timestamp,
        updated_at: timestamp,
        postings,
    };
    transaction.validate()?;
    save_toml(&dest, &transaction)?;
    Ok((id, dest))
}

/// Write a new balance assertion.
pub fn create_assertion(
    root: &Path,
    account_id: Option<String>,
    account_name: Option<String>,
    date: NaiveDate,
    balances: Vec<Balance>,
) -> Result<(String, PathBuf)> {
    if balances.is_empty() {
        return Err(Error::Validation(
            "assertion must declare at least one balance".into(),
        ));
    }
    let accounts = load_accounts(root)?;
    let by_id: HashMap<&str, &Account> = accounts.iter().map(|a| (a.id.as_str(), a)).collect();
    let by_name: HashMap<&str, &Account> = accounts.iter().map(|a| (a.name.as_str(), a)).collect();
    let account = resolve_account(
        account_id.as_deref(),
        account_name.as_deref(),
        &by_id,
        &by_name,
    )?;
    for balance in &balances {
        if !account.allows_currency(&balance.currency) {
            return Err(Error::Validation(format!(
                "asserts {} balance for {}, but that account only allows {}",
                balance.currency,
                account.name,
                account.currencies.join(", "),
            )));
        }
    }

    let (id, dest) = mint_free_id(root, ASSERTIONS_DIR, |id| assertion_relative_path(date, id))?;
    let timestamp = now();
    let assertion = Assertion {
        id: id.clone(),
        account_id: Some(account.id.clone()),
        account_name: Some(account.name.clone()),
        date,
        balances,
        created_at: timestamp,
        updated_at: timestamp,
    };
    save_toml(&dest, &assertion)?;
    Ok((id, dest))
}

/// Write a new price-log entry.
pub fn create_price(
    root: &Path,
    date: NaiveDate,
    base: String,
    quote: String,
    rate: Decimal,
    source: Option<String>,
) -> Result<(String, PathBuf)> {
    if base == quote {
        return Err(Error::Validation(format!(
            "price base and quote are both {base}"
        )));
    }
    let (id, dest) = mint_free_id(root, PRICES_DIR, |id| price_relative_path(date, id))?;
    let timestamp = now();
    let entry = PriceEntry {
        id: id.clone(),
        date,
        base,
        quote,
        rate,
        source,
        created_at: timestamp,
        updated_at: timestamp,
    };
    save_toml(&dest, &entry)?;
    Ok((id, dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn both_generators_produce_seven_chars() {
        assert_eq!(new_id().chars().count(), 7);
        assert_eq!(new_random_id().chars().count(), 7);
    }

    /// `mint_free_id` scopes uniqueness to the kind, not to a month directory.
    ///
    /// A record dated in May takes an id; a record minted in the same
    /// decisecond but dated in June asks `new_id()` for the *same* string,
    /// and its destination path is in a different directory — so only a
    /// kind-wide check can catch it.
    ///
    /// The clock is not controllable, so the round is bracketed with
    /// `new_id()` and only asserted on when it provably stayed inside one
    /// decisecond.
    #[test]
    fn mint_free_id_will_not_reuse_an_id_from_another_month() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let may: NaiveDate = "2026-05-21".parse().unwrap();
        let june: NaiveDate = "2026-06-03".parse().unwrap();

        let mut sampled = 0usize;
        for round in 0..40 {
            let before = new_id();
            let occupied = root.join(transaction_relative_path(may, &before));
            std::fs::create_dir_all(occupied.parent().unwrap()).unwrap();
            std::fs::write(&occupied, "").unwrap();

            let (minted, _) = mint_free_id(root, TRANSACTIONS_DIR, |id| {
                transaction_relative_path(june, id)
            })
            .expect("mint");
            let after = new_id();
            if before == after {
                sampled += 1;
                assert_ne!(
                    minted, before,
                    "minted an id already taken by a record in another month (round {round})"
                );
            }
        }
        assert!(
            sampled > 0,
            "no round stayed inside one decisecond; the collision window was never exercised"
        );
    }

    #[test]
    fn random_ids_vary_in_their_leading_character() {
        // The whole reason accounts use random(): a burst of ids must not
        // share a prefix. 200 draws over a 32-char alphabet hitting only one
        // leading character would be about 1e-300 -- so this is a real signal,
        // not a flaky threshold.
        let leads: HashSet<char> = (0..200)
            .filter_map(|_| new_random_id().chars().next())
            .collect();
        assert!(
            leads.len() > 1,
            "random ids all began with the same character"
        );
    }
}
