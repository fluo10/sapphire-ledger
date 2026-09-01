# Phase 1 (steps 1-3): Write Path and MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give sapphire-ledger a `core::ops` write API with generated ids, rename-safe account references, and an MCP server exposing read and write tools over stdio, so an AI agent can record and query transactions.

**Architecture:** All writes funnel through one `core::ops` module that mints a `grain-id`, resolves the canonical `{kind}/{year}/{MM}/{id}.toml` path, validates, and refuses to overwrite. Records reference accounts by a stable `account_id` with a denormalized `account_name` beside it, so renaming an account never requires rewriting history. `sapphire-ledger-mcp` becomes a library exposing an rmcp `ServerHandler` over stdio, holding a `LedgerState` behind an `Arc<Mutex<_>>`, modelled on `sapphire-journal-mcp`.

**Tech Stack:** Rust 2024 edition, `rmcp` 1.5, `schemars` 1.0, `grain-id`, `rust_decimal`, `chrono`, `toml` 1.1, `sapphire-framework-workspace`, `sapphire-framework-track`.

**Spec:** [`docs/superpowers/specs/2026-09-01-restart-roadmap-design.md`](../specs/2026-09-01-restart-roadmap-design.md)

## Global Constraints

- **Rust edition 2024**, workspace resolver `3`. Member crates use `edition.workspace = true`.
- **Licensing:** every crate stays `MIT OR Apache-2.0` (`license.workspace = true`).
- **Decimals are persisted as TOML strings**, always via `#[serde(with = "rust_decimal::serde::str")]`. TOML has no decimal type.
- **One record, one file.** Never write two records into one file.
- **Record paths** are `{kind}/{year}/{MM}/{id}.toml` for transactions, assertions and prices; `accounts/{Segment}/.../{Leaf}.toml` for accounts.
- **Ids are `grain-id`**, rendered as the 7-character `Display` form. **Accounts use `GrainId::random()`; every other record uses `GrainId::now_unix()`.** Never mix them up — the spec's "Record identity" section explains why each is which.
- **`account_id` is authoritative; `account_name` is never used for matching when an id is present.** A stale `account_name` is **not** an error. A duplicate account id **is**.
- **Writes never overwrite.** Every create refuses if the destination file exists.
- **`Workspace::validate()` never short-circuits** — it returns every issue found.
- **The MCP server logs to stderr only.** stdout carries JSON-RPC and must stay clean.
- **Do not add `rusqlite`** to any crate here. Do not build a SQLite cache.
- **Do not add the `http-server` feature or the server crate** — those are phase 1 step 4, out of scope.

---

## File Structure

**`sapphire-ledger-core/src/`**
- `prices.rs` — **new.** `PriceEntry`, and the inline `Price` moved out of `transaction.rs`.
- `ops.rs` — **new.** The single write path: `new_id`, `new_random_id`, `create_account`, `create_transaction`, `create_assertion`, `create_price`.
- `state.rs` — **new.** `LedgerState`: a loaded `Workspace` with `reload()`.
- `account.rs` — modified: `Account` gains `id`; add `resolve_account`.
- `transaction.rs` — modified: `Price` moves out; `Posting.account` becomes the `account_id` / `account_name` pair.
- `assertion.rs` — modified: `Assertion.account` becomes the same pair.
- `workspace.rs` — modified: `PRICES_DIR`, `price_relative_path`, `init_workspace` creates `prices/`.
- `repository.rs` — modified: `Workspace` gains `prices`.
- `validate.rs` — modified: resolve by id, flag duplicate ids and unresolvable references.
- `lib.rs` — modified: modules and re-exports.

**`sapphire-ledger-mcp/src/`**
- `lib.rs` — modified from a 5-line stub.
- `server.rs` — **new.** Parameter structs, the `#[tool_router]` impl, `ServerHandler`, `prepare_state`, stdio `run`.

**`sapphire-ledger-cli/src/main.rs`** — modified: `Command::Mcp` gains `--init`.

**Manifests** — `Cargo.toml` and the three member manifests.

---

### Task 1: Drop the dead rusqlite pin and add grain-id

The workspace manifest declares `rusqlite = { version = "0.39", features = ["bundled"] }` at `Cargo.toml:23` and **no member crate references it**. `grain-id` needs `rusqlite 0.40.2` behind an optional feature that Cargo still resolves for `links = "sqlite3"` uniqueness, so the stale pin must go first.

**Files:**
- Modify: `Cargo.toml:23`, `sapphire-ledger-core/Cargo.toml`
- Create: `sapphire-ledger-core/src/ops.rs`

**Interfaces:**
- Produces: `ops::new_id() -> String` (time-ordered), `ops::new_random_id() -> String`.

- [ ] **Step 1: Confirm the pin is genuinely unused**

```bash
grep -rn "rusqlite" sapphire-ledger-*/src sapphire-ledger-*/Cargo.toml
```

Expected: no output. If anything matches, stop — the premise of this task is wrong.

- [ ] **Step 2: Edit the workspace manifest**

In `Cargo.toml`, delete the `rusqlite = ...` line from `[workspace.dependencies]` and add:

```toml
grain-id = { version = "0.15", features = ["serde", "schemars"] }
tempfile = "3"
```

Do **not** enable grain-id's `rusqlite` feature. Its default features (`rand`, `std`) are what supply `random()` and `now_unix()`, so leave defaults on.

- [ ] **Step 3: Add the deps to core**

In `sapphire-ledger-core/Cargo.toml`, under `[dependencies]` add `grain-id.workspace = true`, and add:

```toml
[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 4: Write the failing test**

Create `sapphire-ledger-core/src/ops.rs`:

```rust
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn both_generators_produce_seven_chars() {
        assert_eq!(new_id().chars().count(), 7);
        assert_eq!(new_random_id().chars().count(), 7);
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
        assert!(leads.len() > 1, "random ids all began with the same character");
    }
}
```

Add `pub mod ops;` to `sapphire-ledger-core/src/lib.rs`.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-core ops::
```

Expected: 2 passed. A `libsqlite3-sys` `links` failure means the rusqlite pin was not fully removed.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock sapphire-ledger-core/Cargo.toml sapphire-ledger-core/src/ops.rs sapphire-ledger-core/src/lib.rs
git commit -m "feat(core): mint record ids with grain-id, and drop the dead rusqlite pin

Two generators, not one: records named by their id want time ordering,
accounts want leading-character spread for CLI completion."
```

---

### Task 2: Split Price from PriceEntry

`Price` is publicly re-exported at `sapphire-ledger-core/src/lib.rs:24`. Once `schemars` generates MCP tool schemas from these types the name is published, so the split happens before Task 7 exposes anything.

**Files:**
- Create: `sapphire-ledger-core/src/prices.rs`
- Modify: `sapphire-ledger-core/src/transaction.rs:16-21`, `sapphire-ledger-core/src/lib.rs`

**Interfaces:**
- Produces: `prices::Price { value: Decimal, currency: String }`, `prices::PriceEntry { id, date, base, quote, rate, source, created_at, updated_at }`.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-core/src/prices.rs`:

```rust
//! Exchange rates, in two unrelated shapes.
//!
//! [`Price`] is the inline price on a single posting, used to make a
//! cross-currency transaction balance. [`PriceEntry`] is a standalone record
//! in the price log, used to report in a base currency at a past date. They
//! share a domain but not a lifecycle, and one of them is part of the
//! published MCP tool schema — so they are separate types.

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The inline price on a posting: "this amount, valued at `value` `currency`".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    pub currency: String,
}

/// One observed exchange rate: `1 base = rate quote` on `date`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceEntry {
    pub id: String,
    pub date: NaiveDate,
    pub base: String,
    pub quote: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub rate: Decimal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_entry_round_trips_through_toml() {
        let toml_text = r#"
id = "0a1b2c3"
date = "2026-05-21"
base = "USD"
quote = "JPY"
rate = "150.5"
source = "manual"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"
"#;
        let entry: PriceEntry = toml::from_str(toml_text).expect("parse");
        assert_eq!(entry.rate.to_string(), "150.5");

        let rendered = toml::to_string_pretty(&entry).expect("serialize");
        let round_tripped: PriceEntry = toml::from_str(&rendered).expect("reparse");
        assert_eq!(entry, round_tripped);
    }

    #[test]
    fn price_entry_omits_absent_source() {
        let entry = PriceEntry {
            id: "0a1b2c3".into(),
            date: "2026-05-21".parse().unwrap(),
            base: "USD".into(),
            quote: "JPY".into(),
            rate: "150".parse().unwrap(),
            source: None,
            created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
            updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        };
        let rendered = toml::to_string_pretty(&entry).expect("serialize");
        assert!(!rendered.contains("source"), "got: {rendered}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-core prices::
```

Expected: FAIL — `prices` is not a declared module.

- [ ] **Step 3: Wire the module and move the type**

Add `pub mod prices;` to `sapphire-ledger-core/src/lib.rs`.

In `sapphire-ledger-core/src/transaction.rs`, **delete** the `Price` struct (lines 16-21) and put in its place:

```rust
pub use crate::prices::Price;
```

- [ ] **Step 4: Fix the crate re-exports**

In `sapphire-ledger-core/src/lib.rs`:

```rust
pub use prices::{Price, PriceEntry};
pub use transaction::{Posting, Transaction, TransactionStatus};
```

`Price` must be re-exported from exactly one place — leaving it in both lists is a duplicate-import compile error.

- [ ] **Step 5: Run the suite and commit**

```bash
cargo test -p sapphire-ledger-core
git add sapphire-ledger-core/src/prices.rs sapphire-ledger-core/src/transaction.rs sapphire-ledger-core/src/lib.rs
git commit -m "refactor(core): separate the inline posting price from the price-log record

Price is about to become part of the published MCP tool schema; renaming it
after that ships would be a schema break."
```

---

### Task 3: Account identity and rename-safe references

`Account` is the only record with no id, and both `Posting` and `Assertion` reference accounts by name — so renaming an account today means rewriting every file that mentions it. Under record-level sync that rewrite is not atomic and races with concurrent writes. This task makes the id the link.

**Files:**
- Modify: `sapphire-ledger-core/src/account.rs`, `transaction.rs`, `assertion.rs`, `validate.rs`, `lib.rs`
- Test: `sapphire-ledger-core/tests/account_refs.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `Account.id: String` (first field).
  - `Posting.account_id: Option<String>`, `Posting.account_name: Option<String>` — replacing `Posting.account`.
  - `Assertion.account_id: Option<String>`, `Assertion.account_name: Option<String>` — replacing `Assertion.account`.
  - `account::resolve_account<'a>(id: Option<&str>, name: Option<&str>, by_id: &HashMap<&str, &'a Account>, by_name: &HashMap<&str, &'a Account>) -> Result<&'a Account>`
  - `account::describe_ref(id: Option<&str>, name: Option<&str>) -> String` — for error messages.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-core/tests/account_refs.rs`:

```rust
use sapphire_ledger_core::{Account, AccountType, Assertion, Balance, Posting, Transaction};

fn account(id: &str, name: &str) -> Account {
    Account {
        id: id.to_string(),
        name: name.to_string(),
        account_type: AccountType::Expense,
        currencies: vec![],
        opened_at: "2026-01-01".parse().unwrap(),
        closed_at: None,
        description: None,
    }
}

fn posting(id: Option<&str>, name: Option<&str>, amount: &str) -> Posting {
    Posting {
        account_id: id.map(str::to_string),
        account_name: name.map(str::to_string),
        amount: amount.parse().unwrap(),
        currency: "JPY".into(),
        price: None,
        memo: None,
    }
}

fn transaction(postings: Vec<Posting>) -> Transaction {
    Transaction {
        id: "tx00001".into(),
        date: "2026-05-21".parse().unwrap(),
        narration: "test".into(),
        payee: None,
        tags: vec![],
        status: None,
        created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        postings,
    }
}

fn workspace(accounts: Vec<Account>, transactions: Vec<Transaction>, assertions: Vec<Assertion>)
    -> sapphire_ledger_core::Workspace
{
    sapphire_ledger_core::Workspace {
        root: std::path::PathBuf::from("/nonexistent"),
        config: sapphire_ledger_core::Config {
            schema_version: sapphire_ledger_core::CURRENT_SCHEMA_VERSION,
            base_currency: "JPY".into(),
            cache: Default::default(),
        },
        accounts,
        transactions,
        assertions,
        prices: vec![],
    }
}

#[test]
fn a_posting_resolves_by_id_even_when_the_name_is_stale() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Groceries")],
        vec![transaction(vec![
            posting(Some("acct001"), Some("Expenses:Food"), "1200"),
            posting(Some("acct001"), Some("Expenses:Food"), "-1200"),
        ])],
        vec![],
    );
    assert!(
        ws.validate().is_empty(),
        "a renamed account must not make old postings invalid: {:?}",
        ws.validate()
    );
}

#[test]
fn a_posting_resolves_by_name_when_no_id_is_given() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(None, Some("Expenses:Food"), "1200"),
            posting(None, Some("Expenses:Food"), "-1200"),
        ])],
        vec![],
    );
    assert!(ws.validate().is_empty(), "{:?}", ws.validate());
}

#[test]
fn a_posting_with_neither_reference_is_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(None, None, "1200"),
            posting(None, None, "-1200"),
        ])],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("no account reference")),
        "{issues:?}"
    );
}

#[test]
fn an_unknown_account_id_is_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(Some("nosuch"), Some("Expenses:Food"), "1200"),
            posting(Some("acct001"), None, "-1200"),
        ])],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("undefined account")),
        "an id that resolves to nothing must fail even when the name would have \
         resolved -- the id is authoritative: {issues:?}"
    );
}

#[test]
fn duplicate_account_ids_are_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food"), account("acct001", "Expenses:Other")],
        vec![],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("duplicate account id")),
        "a rename racing under sync can put one id at two paths: {issues:?}"
    );
}

#[test]
fn an_assertion_resolves_by_id_too() {
    let ws = workspace(
        vec![account("acct001", "Assets:Cash:JPY")],
        vec![],
        vec![Assertion {
            id: "as00001".into(),
            account_id: Some("acct001".into()),
            account_name: Some("Assets:Old:Name".into()),
            date: "2026-05-31".parse().unwrap(),
            balances: vec![Balance { amount: "5000".parse().unwrap(), currency: "JPY".into() }],
            created_at: "2026-05-31T23:59:00+09:00".parse().unwrap(),
            updated_at: "2026-05-31T23:59:00+09:00".parse().unwrap(),
        }],
    );
    assert!(ws.validate().is_empty(), "{:?}", ws.validate());
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-core --test account_refs
```

Expected: FAIL — `Account` has no `id`, `Posting` has no `account_id`.

- [ ] **Step 3: Give Account an id and add the resolver**

In `sapphire-ledger-core/src/account.rs`, add `id` as the first field of `Account`:

```rust
pub struct Account {
    pub id: String,
    pub name: String,
    // ... existing fields unchanged
```

and append to the file:

```rust
use std::collections::HashMap;

/// Render an account reference for an error message, whichever half was given.
pub fn describe_ref(id: Option<&str>, name: Option<&str>) -> String {
    match (id, name) {
        (Some(id), Some(name)) => format!("{name} ({id})"),
        (Some(id), None) => id.to_string(),
        (None, Some(name)) => name.to_string(),
        (None, None) => "<no account reference>".to_string(),
    }
}

/// Resolve an account reference.
///
/// The id is authoritative: when it is present the name is not consulted at
/// all, which is what lets a rename leave old records alone. A name-only
/// reference is resolved by name, so hand-written TOML stays valid.
pub fn resolve_account<'a>(
    id: Option<&str>,
    name: Option<&str>,
    by_id: &HashMap<&str, &'a Account>,
    by_name: &HashMap<&str, &'a Account>,
) -> Result<&'a Account> {
    match (id, name) {
        (Some(id), _) => by_id.get(id).copied().ok_or_else(|| {
            Error::Validation(format!("undefined account id {id}"))
        }),
        (None, Some(name)) => by_name.get(name).copied().ok_or_else(|| {
            Error::Validation(format!("undefined account {name}"))
        }),
        (None, None) => Err(Error::Validation(
            "posting or assertion has no account reference".into(),
        )),
    }
}
```

- [ ] **Step 4: Replace the name references on Posting and Assertion**

In `sapphire-ledger-core/src/transaction.rs`, replace `pub account: String,` on `Posting` with:

```rust
    /// The authoritative link. Survives a rename of the account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// A denormalized copy of the account's name, for whoever reads the raw
    /// file. Never used for matching when `account_id` is set, and allowed to
    /// go stale after a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
```

In `sapphire-ledger-core/src/assertion.rs`, replace `pub account: String,` on `Assertion` with the same two fields and the same doc comments.

- [ ] **Step 5: Rewrite validation to resolve by id**

Replace the body of `Workspace::validate` in `sapphire-ledger-core/src/validate.rs`:

```rust
    pub fn validate(&self) -> Vec<ValidationIssue> {
        use crate::account::{describe_ref, resolve_account};

        let mut issues = Vec::new();
        let mut by_id: HashMap<&str, &Account> = HashMap::new();
        let mut by_name: HashMap<&str, &Account> = HashMap::new();

        for account in &self.accounts {
            by_name.insert(account.name.as_str(), account);
            // A rename is a file move; if that races under record-level sync,
            // one id can end up at two paths. Unlike a stale name, that is
            // genuinely broken.
            if by_id.insert(account.id.as_str(), account).is_some() {
                issues.push(ValidationIssue {
                    message: format!("duplicate account id {}", account.id),
                    transaction_id: None,
                    assertion_id: None,
                    account: Some(account.name.clone()),
                });
            }
        }

        for tx in &self.transactions {
            if let Err(err) = tx.validate() {
                issues.push(ValidationIssue {
                    message: render(&err),
                    transaction_id: Some(tx.id.clone()),
                    assertion_id: None,
                    account: None,
                });
            }

            for posting in &tx.postings {
                let id = posting.account_id.as_deref();
                let name = posting.account_name.as_deref();
                match resolve_account(id, name, &by_id, &by_name) {
                    Err(err) => issues.push(ValidationIssue {
                        message: format!("transaction {}: {}", tx.id, render(&err)),
                        transaction_id: Some(tx.id.clone()),
                        assertion_id: None,
                        account: Some(describe_ref(id, name)),
                    }),
                    Ok(account) => {
                        if !account.allows_currency(&posting.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "transaction {} posts {} to {}, but that account only allows {}",
                                    tx.id,
                                    posting.currency,
                                    account.name,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: Some(tx.id.clone()),
                                assertion_id: None,
                                account: Some(account.name.clone()),
                            });
                        }
                    }
                }
            }
        }

        for assertion in &self.assertions {
            let id = assertion.account_id.as_deref();
            let name = assertion.account_name.as_deref();
            match resolve_account(id, name, &by_id, &by_name) {
                Err(err) => issues.push(ValidationIssue {
                    message: format!("assertion {}: {}", assertion.id, render(&err)),
                    transaction_id: None,
                    assertion_id: Some(assertion.id.clone()),
                    account: Some(describe_ref(id, name)),
                }),
                Ok(account) => {
                    for balance in &assertion.balances {
                        if !account.allows_currency(&balance.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "assertion {} asserts {} balance for {}, but that account only allows {}",
                                    assertion.id,
                                    balance.currency,
                                    account.name,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: None,
                                assertion_id: Some(assertion.id.clone()),
                                account: Some(account.name.clone()),
                            });
                        }
                    }
                }
            }
        }

        issues
    }
```

The error text must contain "no account reference", "undefined account" and "duplicate account id" — the tests match on those substrings, and so will the agent reading tool output.

- [ ] **Step 6: Export the new helpers**

In `sapphire-ledger-core/src/lib.rs`, extend the account re-export:

```rust
pub use account::{Account, AccountType, account_name_segments, describe_ref, resolve_account};
```

- [ ] **Step 7: Run the tests**

```bash
cargo test -p sapphire-ledger-core
```

Expected: the six new tests pass. `undefined account id nosuch` contains "undefined account", which is what `an_unknown_account_id_is_an_error` matches.

- [ ] **Step 8: Commit**

```bash
git add sapphire-ledger-core/src/account.rs sapphire-ledger-core/src/transaction.rs sapphire-ledger-core/src/assertion.rs sapphire-ledger-core/src/validate.rs sapphire-ledger-core/src/lib.rs sapphire-ledger-core/tests/account_refs.rs
git commit -m "feat(core): link postings and assertions to accounts by id

Renaming an account previously meant rewriting every file that named it --
a multi-file write that record-level, last-writer-wins sync cannot make
atomic. The id is now the link, the name rides along for human readers, and
a stale name is explicitly not an error."
```

---

### Task 4: Price paths and workspace loading

**Files:**
- Modify: `sapphire-ledger-core/src/workspace.rs`, `repository.rs`, `lib.rs`
- Test: `sapphire-ledger-core/tests/workspace_prices.rs`

**Interfaces:**
- Consumes: `PriceEntry` (Task 2).
- Produces: `workspace::PRICES_DIR`, `workspace::price_relative_path(date: NaiveDate, id: &str) -> PathBuf`, and `Workspace.prices: Vec<PriceEntry>`.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-core/tests/workspace_prices.rs`:

```rust
use sapphire_ledger_core::{init_workspace, load_workspace, price_relative_path, save_toml, PriceEntry};

#[test]
fn init_creates_the_prices_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_workspace(dir.path(), "JPY").expect("init");
    assert!(dir.path().join("prices").is_dir());
}

#[test]
fn price_path_is_year_month_id() {
    let date = "2026-05-21".parse().unwrap();
    assert_eq!(
        price_relative_path(date, "0a1b2c3"),
        std::path::Path::new("prices/2026/05/0a1b2c3.toml")
    );
}

#[test]
fn load_workspace_reads_price_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_workspace(dir.path(), "JPY").expect("init");

    let entry = PriceEntry {
        id: "0a1b2c3".into(),
        date: "2026-05-21".parse().unwrap(),
        base: "USD".into(),
        quote: "JPY".into(),
        rate: "150".parse().unwrap(),
        source: Some("manual".into()),
        created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
    };
    save_toml(&dir.path().join(price_relative_path(entry.date, &entry.id)), &entry).expect("save");

    let ws = load_workspace(dir.path()).expect("load");
    assert_eq!(ws.prices.len(), 1);
    assert_eq!(ws.prices[0].base, "USD");
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-core --test workspace_prices
```

Expected: FAIL — `price_relative_path` does not exist, `Workspace` has no `prices`.

- [ ] **Step 3: Add the constant, the path helper, and the directory**

In `sapphire-ledger-core/src/workspace.rs`, beside `ASSERTIONS_DIR`:

```rust
pub const PRICES_DIR: &str = "prices";
```

beside `assertion_relative_path`:

```rust
/// Relative path for a price-log entry: `prices/{year}/{MM}/{id}.toml`.
pub fn price_relative_path(date: NaiveDate, id: &str) -> PathBuf {
    PathBuf::from(PRICES_DIR)
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{id}.{TOML_EXTENSION}"))
}
```

and in `init_workspace`, beside the other `create_dir_all` calls:

```rust
    fs::create_dir_all(target.join(PRICES_DIR))?;
```

- [ ] **Step 4: Load them**

In `sapphire-ledger-core/src/repository.rs`: add `use crate::prices::PriceEntry;`, add `PRICES_DIR` to the `crate::workspace::{...}` import, add `pub prices: Vec<PriceEntry>,` to `Workspace`, and before the `Ok(Workspace { ... })`:

```rust
    let prices = walk_toml_files(&root.join(PRICES_DIR))?
        .iter()
        .map(|p| load_toml::<PriceEntry>(p))
        .collect::<Result<Vec<_>>>()?;
```

then add `prices,` to the struct literal.

- [ ] **Step 5: Export, test, commit**

Add `price_relative_path` and `PRICES_DIR` to the `pub use workspace::{...}` list in `lib.rs`.

```bash
cargo test -p sapphire-ledger-core
git add sapphire-ledger-core/src/workspace.rs sapphire-ledger-core/src/repository.rs sapphire-ledger-core/src/lib.rs sapphire-ledger-core/tests/workspace_prices.rs
git commit -m "feat(core): store and load price-log entries"
```

---

### Task 5: The create functions

Every create mints an id, resolves the canonical path, validates, and refuses to overwrite. Account creation additionally checks its random id against the existing accounts, since an account's id is not its filename and the file-exists check cannot see it. Transaction and assertion creation resolve name-only references to ids, so records always land on disk carrying the authoritative link.

**Files:**
- Modify: `sapphire-ledger-core/src/ops.rs`
- Test: `sapphire-ledger-core/tests/ops_create.rs`

**Interfaces:**
- Consumes: `new_id`, `new_random_id` (Task 1); the account-ref fields (Task 3); `price_relative_path` (Task 4).
- Produces:
  - `ops::create_account(root: &Path, name: String, account_type: AccountType, currencies: Vec<String>, opened_at: NaiveDate, description: Option<String>) -> Result<(String, PathBuf)>`
  - `ops::create_transaction(root: &Path, date: NaiveDate, narration: String, payee: Option<String>, tags: Vec<String>, status: Option<TransactionStatus>, postings: Vec<Posting>) -> Result<(String, PathBuf)>`
  - `ops::create_assertion(root: &Path, account_id: Option<String>, account_name: Option<String>, date: NaiveDate, balances: Vec<Balance>) -> Result<(String, PathBuf)>`
  - `ops::create_price(root: &Path, date: NaiveDate, base: String, quote: String, rate: Decimal, source: Option<String>) -> Result<(String, PathBuf)>`

- [ ] **Step 1: Write the failing tests**

Create `sapphire-ledger-core/tests/ops_create.rs`:

```rust
use rust_decimal::Decimal;
use sapphire_ledger_core::{ops, AccountType, Balance, Posting};

fn ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");
    dir
}

fn add_account(root: &std::path::Path, name: &str, ty: AccountType) -> String {
    let (id, _) = ops::create_account(
        root,
        name.to_string(),
        ty,
        vec![],
        "2026-01-01".parse().unwrap(),
        None,
    )
    .expect("create_account");
    id
}

fn posting_by_name(name: &str, amount: &str) -> Posting {
    Posting {
        account_id: None,
        account_name: Some(name.to_string()),
        amount: amount.parse::<Decimal>().unwrap(),
        currency: "JPY".into(),
        price: None,
        memo: None,
    }
}

#[test]
fn create_account_writes_the_hierarchical_path_and_mints_an_id() {
    let dir = ws();
    let (id, dest) = ops::create_account(
        dir.path(),
        "Assets:Cash:JPY".into(),
        AccountType::Asset,
        vec![],
        "2026-01-01".parse().unwrap(),
        None,
    )
    .expect("create");
    assert_eq!(id.chars().count(), 7);
    assert!(dest.ends_with("accounts/Assets/Cash/JPY.toml"), "got {}", dest.display());
    assert!(std::fs::read_to_string(&dest).unwrap().contains(&id));
}

#[test]
fn create_account_refuses_a_duplicate_name() {
    let dir = ws();
    add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);
    let err = ops::create_account(
        dir.path(),
        "Assets:Cash:JPY".into(),
        AccountType::Asset,
        vec![],
        "2026-01-01".parse().unwrap(),
        None,
    )
    .expect_err("must refuse");
    assert!(err.to_string().contains("already exists"), "got: {err}");
}

#[test]
fn create_account_rejects_a_malformed_name() {
    let dir = ws();
    let err = ops::create_account(
        dir.path(),
        "Assets::Cash".into(),
        AccountType::Asset,
        vec![],
        "2026-01-01".parse().unwrap(),
        None,
    )
    .expect_err("must reject");
    assert!(err.to_string().contains("empty segment"), "got: {err}");
}

#[test]
fn create_transaction_resolves_names_to_ids_on_disk() {
    let dir = ws();
    let food = add_account(dir.path(), "Expenses:Food", AccountType::Expense);
    add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);

    let (id, dest) = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "イオン買い物".into(),
        Some("イオン".into()),
        vec!["grocery".into()],
        None,
        vec![posting_by_name("Expenses:Food", "1200"), posting_by_name("Assets:Cash:JPY", "-1200")],
    )
    .expect("create");

    assert!(dest.ends_with(format!("transactions/2026/05/{id}.toml")), "got {}", dest.display());
    let written = std::fs::read_to_string(&dest).expect("read");
    assert!(
        written.contains(&format!("account_id = \"{food}\"")),
        "a name-only posting must be written with its resolved id: {written}"
    );
    assert!(written.contains("account_name = \"Expenses:Food\""));
}

#[test]
fn create_transaction_rejects_an_unknown_account() {
    let dir = ws();
    add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);
    let err = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "orphan".into(),
        None,
        vec![],
        None,
        vec![posting_by_name("Expenses:Nowhere", "1200"), posting_by_name("Assets:Cash:JPY", "-1200")],
    )
    .expect_err("must reject");
    assert!(err.to_string().contains("undefined account"), "got: {err}");
}

#[test]
fn create_transaction_rejects_an_unbalanced_entry_and_writes_nothing() {
    let dir = ws();
    add_account(dir.path(), "Expenses:Food", AccountType::Expense);
    add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);

    let err = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "wrong".into(),
        None,
        vec![],
        None,
        vec![posting_by_name("Expenses:Food", "1200"), posting_by_name("Assets:Cash:JPY", "-999")],
    )
    .expect_err("must reject");
    assert!(err.to_string().contains("does not balance"), "got: {err}");

    let month = dir.path().join("transactions/2026/05");
    assert_eq!(
        std::fs::read_dir(&month).map(|d| d.count()).unwrap_or(0),
        0,
        "a rejected transaction must leave no file behind"
    );
}

#[test]
fn create_assertion_and_price_produce_dated_paths() {
    let dir = ws();
    let cash = add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);

    let (aid, apath) = ops::create_assertion(
        dir.path(),
        Some(cash),
        None,
        "2026-05-31".parse().unwrap(),
        vec![Balance { amount: "5000".parse().unwrap(), currency: "JPY".into() }],
    )
    .expect("assertion");
    assert!(apath.ends_with(format!("assertions/2026/05/{aid}.toml")), "got {}", apath.display());

    let (pid, ppath) = ops::create_price(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "USD".into(),
        "JPY".into(),
        "150".parse().unwrap(),
        Some("manual".into()),
    )
    .expect("price");
    assert!(ppath.ends_with(format!("prices/2026/05/{pid}.toml")), "got {}", ppath.display());
}

#[test]
fn created_records_reload_and_validate_clean() {
    let dir = ws();
    add_account(dir.path(), "Expenses:Food", AccountType::Expense);
    add_account(dir.path(), "Assets:Cash:JPY", AccountType::Asset);
    ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "イオン買い物".into(),
        None,
        vec![],
        None,
        vec![posting_by_name("Expenses:Food", "1200"), posting_by_name("Assets:Cash:JPY", "-1200")],
    )
    .expect("create");

    let loaded = sapphire_ledger_core::load_workspace(dir.path()).expect("load");
    assert_eq!(loaded.accounts.len(), 2);
    assert_eq!(loaded.transactions.len(), 1);
    assert!(loaded.validate().is_empty(), "issues: {:?}", loaded.validate());
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-core --test ops_create
```

Expected: FAIL — no `create_*` functions.

- [ ] **Step 3: Implement the create functions**

Append to `sapphire-ledger-core/src/ops.rs`, between `new_random_id` and the test module:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::account::{resolve_account, Account, AccountType};
use crate::assertion::{Assertion, Balance};
use crate::error::{Error, Result};
use crate::prices::PriceEntry;
use crate::repository::{load_toml, save_toml, walk_toml_files};
use crate::transaction::{Posting, Transaction, TransactionStatus};
use crate::workspace::{
    account_relative_path, assertion_relative_path, price_relative_path,
    transaction_relative_path, ACCOUNTS_DIR,
};

/// How many times to re-mint an id when the destination is already taken.
///
/// For time-ordered ids a collision means "another record was created in this
/// same tenth of a second"; re-minting after the clock advances is enough. For
/// an account's random id a collision is astronomically unlikely but must
/// still terminate.
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

/// Mint a time-ordered id whose destination file does not exist yet.
fn mint_free_id<F>(root: &Path, relative: F) -> Result<(String, PathBuf)>
where
    F: Fn(&str) -> PathBuf,
{
    for _ in 0..ID_ATTEMPTS {
        let id = new_id();
        let dest = root.join(relative(&id));
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
/// The path comes from the name, so a duplicate name is a hard error. The id
/// is random and is *not* in the path, so the refuse-to-overwrite check cannot
/// see an id collision — that is checked against the existing accounts here.
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
    let taken: std::collections::HashSet<&str> =
        existing.iter().map(|a| a.id.as_str()).collect();

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

    let (id, dest) = mint_free_id(root, |id| transaction_relative_path(date, id))?;
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

    let (id, dest) = mint_free_id(root, |id| assertion_relative_path(date, id))?;
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
    let (id, dest) = mint_free_id(root, |id| price_relative_path(date, id))?;
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
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p sapphire-ledger-core
```

Expected: all pass. Note `create_transaction` resolves and validates before `save_toml`, which is what `create_transaction_rejects_an_unbalanced_entry_and_writes_nothing` checks.

- [ ] **Step 5: Commit**

```bash
git add sapphire-ledger-core/src/ops.rs sapphire-ledger-core/tests/ops_create.rs
git commit -m "feat(core): one write path for every record kind

Mints an id, resolves account references to ids, validates, refuses to
overwrite. A record never reaches disk carrying only an account name -- the
name is the half allowed to go stale."
```

---

### Task 6: LedgerState and the framework dependency

Per the spec's scope note, this task introduces the state object and the dependency **only**. It builds no index: nothing in phase 1 steps 1-3 has a consumer for one.

**Files:**
- Create: `sapphire-ledger-core/src/state.rs`
- Modify: `sapphire-ledger-core/src/lib.rs`, `sapphire-ledger-core/Cargo.toml`, `Cargo.toml`
- Test: `sapphire-ledger-core/tests/state_reload.rs`

**Interfaces:**
- Produces: `LedgerState::open(root: &Path) -> Result<Self>`, `LedgerState::find(start: &Path) -> Result<Self>`, `LedgerState::workspace(&self) -> &Workspace`, `LedgerState::root(&self) -> &Path`, `LedgerState::reload(&mut self) -> Result<()>`, and `sapphire_ledger_core::LEDGER_CTX`.

- [ ] **Step 1: Add the framework dependencies**

In the workspace `Cargo.toml` under `[workspace.dependencies]`:

```toml
sapphire-workspace = { package = "sapphire-framework-workspace", git = "https://github.com/fluo10/sapphire-framework", branch = "main", default-features = false }
sapphire-track = { package = "sapphire-framework-track", git = "https://github.com/fluo10/sapphire-framework", branch = "main" }
```

The `package = ` alias keeps the short extern name, matching `sapphire-journal-core`.

In `sapphire-ledger-core/Cargo.toml`, add `sapphire-workspace.workspace = true` under `[dependencies]` and:

```toml
[features]
default = ["redb-store"]
redb-store = ["sapphire-workspace/redb-store"]
```

`sapphire-track` is declared at the workspace level for later index work but wired into no crate here — nothing consumes it yet.

- [ ] **Step 2: Write the failing test**

Create `sapphire-ledger-core/tests/state_reload.rs`:

```rust
use sapphire_ledger_core::{ops, AccountType, LedgerState};

#[test]
fn reload_picks_up_a_record_written_after_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");

    let mut state = LedgerState::open(dir.path()).expect("open");
    assert_eq!(state.workspace().accounts.len(), 0);

    ops::create_account(
        dir.path(),
        "Assets:Cash:JPY".into(),
        AccountType::Asset,
        vec![],
        "2026-01-01".parse().unwrap(),
        None,
    )
    .expect("create");

    assert_eq!(state.workspace().accounts.len(), 0, "open() takes a snapshot");
    state.reload().expect("reload");
    assert_eq!(state.workspace().accounts.len(), 1, "reload() must see the new file");
}

#[test]
fn find_walks_upward_to_the_workspace_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");
    let nested = dir.path().join("transactions/2026/05");
    std::fs::create_dir_all(&nested).expect("mkdir");

    let state = LedgerState::find(&nested).expect("find");
    assert_eq!(
        state.root().canonicalize().unwrap(),
        dir.path().canonicalize().unwrap()
    );
}
```

- [ ] **Step 3: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-core --test state_reload
```

Expected: FAIL — `LedgerState` does not exist.

- [ ] **Step 4: Implement LedgerState**

Create `sapphire-ledger-core/src/state.rs`:

```rust
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
use crate::repository::{load_workspace, Workspace};
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
        Ok(Self { root: root.to_path_buf(), workspace })
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
```

- [ ] **Step 5: Declare the module, the app context, and the re-export**

In `sapphire-ledger-core/src/lib.rs`, add `pub mod state;`, then:

```rust
pub use state::LedgerState;

/// Process-wide application context, naming the cache and data directories
/// the framework uses. Mirrors `JOURNAL_CTX` in sapphire-journal.
pub static LEDGER_CTX: sapphire_workspace::AppContext =
    sapphire_workspace::AppContext::new("sapphire-ledger");
```

`AppContext::new` is `const` (`crates/sapphire-framework-workspace/src/context.rs:56`), so the static compiles as written.

- [ ] **Step 6: Test and commit**

```bash
cargo test -p sapphire-ledger-core
git add Cargo.toml Cargo.lock sapphire-ledger-core/Cargo.toml sapphire-ledger-core/src/state.rs sapphire-ledger-core/src/lib.rs sapphire-ledger-core/tests/state_reload.rs
git commit -m "feat(core): hold an open ledger in a LedgerState, on the framework

No index yet -- nothing in this phase searches. The point is that the state
object and the framework dependency exist while ledger is still small enough
that introducing them costs one struct."
```

The first framework build pulls a git dependency and will be slow.

---

### Task 7: The MCP server — read tools

**Files:**
- Create: `sapphire-ledger-mcp/src/server.rs`
- Modify: `sapphire-ledger-mcp/src/lib.rs`, `sapphire-ledger-mcp/Cargo.toml`

**Interfaces:**
- Consumes: `LedgerState` (Task 6), `Workspace` fields (Task 4), account-ref fields (Task 3).
- Produces: `SapphireLedgerServer::new(state)`, `::from_shared(Arc<Mutex<LedgerState>>)`, `::shared_state()`, `::with_write_observer()`, and `prepare_state(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<LedgerState>`.

- [ ] **Step 1: Set up the crate manifest**

Replace the `[dependencies]` block of `sapphire-ledger-mcp/Cargo.toml` and add a features section:

```toml
[features]
default = ["redb-store"]
redb-store = ["sapphire-ledger-core/redb-store"]

[dependencies]
sapphire-ledger-core = { path = "../sapphire-ledger-core", version = "0.1.0", default-features = false }
rmcp.workspace = true
tokio = { workspace = true, features = ["full"] }
serde.workspace = true
serde_json.workspace = true
schemars.workspace = true
chrono.workspace = true
rust_decimal.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `sapphire-ledger-mcp/src/server.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a throwaway server over a freshly initialized ledger. Hold the
    /// `TempDir` for the test's duration — dropping it removes the directory
    /// the open state points at.
    fn test_server() -> (tempfile::TempDir, SapphireLedgerServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = prepare_state(Some(dir.path()), true).expect("init test ledger");
        (dir, SapphireLedgerServer::new(state))
    }

    #[test]
    fn list_accounts_is_empty_on_a_fresh_ledger() {
        let (_dir, server) = test_server();
        let json = server.list_accounts(Parameters(EmptyParams {})).expect("ok");
        assert_eq!(json.trim(), "[]");
    }

    /// Zero-argument tools must advertise a top-level `type: object`, or
    /// Anthropic rejects the tool with
    /// `tools.<N>.custom.input_schema.type: Field required`.
    #[test]
    fn every_tool_input_schema_declares_object_type() {
        for tool in SapphireLedgerServer::tool_router().list_all() {
            let schema = serde_json::to_value(&tool.input_schema).expect("schema to json");
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "tool {} has a non-object input schema: {schema}",
                tool.name
            );
        }
    }

    #[test]
    fn validate_workspace_reports_an_undefined_account() {
        let (dir, server) = test_server();
        // Written straight to disk: ops::create_transaction would refuse it,
        // which is the point -- this is the hand-edited-file case that
        // validate_workspace exists to catch.
        let path = dir.path().join("transactions/2026/05/tx00001.toml");
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            r#"
id = "tx00001"
date = "2026-05-21"
narration = "orphan"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Nowhere"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Nowhere"
amount = "-10"
currency = "JPY"
"#,
        )
        .expect("write");

        let json = server.validate_workspace(Parameters(EmptyParams {})).expect("ok");
        assert!(json.contains("undefined account"), "got: {json}");
    }
}
```

- [ ] **Step 3: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-mcp
```

Expected: FAIL — nothing in the test module resolves.

- [ ] **Step 4: Implement the server and the read tools**

Prepend to `sapphire-ledger-mcp/src/server.rs`, above the test module:

```rust
//! MCP server logic for sapphire-ledger.
//!
//! Modelled on `sapphire-journal-mcp`: one server struct holding an
//! `Arc<Mutex<LedgerState>>`, tools declared with rmcp's `#[tool]` macro, and
//! a stdio entry point. The HTTP transport arrives later, with
//! `sapphire-ledger-server`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    tool, tool_router,
    transport::stdio,
};
use sapphire_ledger_core::{ops, LedgerState};
use serde::Deserialize;

/// Called after a tool writes, with every path the write touched.
///
/// One call is one batch: a write producing several files reports them
/// together, so a syncing receiver never sees half of a change.
pub type WriteObserver = Arc<dyn Fn(&[PathBuf]) + Send + Sync>;

#[derive(Clone)]
pub struct SapphireLedgerServer {
    state: Arc<Mutex<LedgerState>>,
    tool_router: ToolRouter<Self>,
    write_observer: Option<WriteObserver>,
}

impl std::fmt::Debug for SapphireLedgerServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SapphireLedgerServer").finish_non_exhaustive()
    }
}

impl SapphireLedgerServer {
    pub fn new(state: LedgerState) -> Self {
        Self::from_shared(Arc::new(Mutex::new(state)))
    }

    /// Build a server sharing an existing state handle. The HTTP transport
    /// spawns one server per session, but all of them must see one ledger.
    pub fn from_shared(state: Arc<Mutex<LedgerState>>) -> Self {
        Self { state, tool_router: Self::tool_router(), write_observer: None }
    }

    pub fn shared_state(&self) -> Arc<Mutex<LedgerState>> {
        Arc::clone(&self.state)
    }

    /// Set where post-write notifications go. Unused by the stdio transport.
    pub fn with_write_observer(mut self, observer: WriteObserver) -> Self {
        self.write_observer = Some(observer);
        self
    }

    fn notify_write(&self, paths: &[PathBuf]) {
        if let Some(observer) = &self.write_observer {
            observer(paths);
        }
    }

    /// Take the state lock, recovering it if an earlier panic poisoned it.
    ///
    /// `.lock().unwrap()` would turn one panic under this mutex into a
    /// permanently dead server. `LedgerState`'s invariants do not depend on
    /// the previous holder finishing: it holds a snapshot that `reload()`
    /// rebuilds from scratch. So warn once and carry on.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, LedgerState> {
        if self.state.is_poisoned() {
            tracing::warn!(
                "ledger state mutex was poisoned by an earlier panic; recovering it rather \
                 than failing every tool call for the rest of the process"
            );
        }
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// ── parameter structs ─────────────────────────────────────────────────────────

/// Explicit empty parameter object for zero-argument tools.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTransactionParams {
    /// The transaction's 7-character id.
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryPostingsParams {
    /// Account id, or account name. Either is accepted.
    pub account: Option<String>,
    /// Currency code, e.g. `JPY`.
    pub currency: Option<String>,
    /// Inclusive lower bound, `YYYY-MM-DD`.
    pub date_from: Option<String>,
    /// Inclusive upper bound, `YYYY-MM-DD`.
    pub date_to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryPricesParams {
    pub base: Option<String>,
    pub quote: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

fn parse_date(raw: &str) -> anyhow::Result<chrono::NaiveDate> {
    raw.parse::<chrono::NaiveDate>()
        .with_context(|| format!("not a YYYY-MM-DD date: {raw}"))
}

/// One posting flattened with its transaction's context, which is what a
/// caller asking "what happened to this account" actually wants.
#[derive(serde::Serialize)]
struct PostingHit {
    transaction_id: String,
    date: chrono::NaiveDate,
    narration: String,
    account_id: Option<String>,
    account_name: Option<String>,
    amount: String,
    currency: String,
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl SapphireLedgerServer {
    #[tool(description = "List every account with its id, name, type, allowed currencies \
        and open date. The id is the stable handle: it survives a rename, and other \
        tools accept it wherever they accept a name.")]
    fn list_accounts(&self, Parameters(_): Parameters<EmptyParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let guard = self.lock_state();
            Ok(serde_json::to_string_pretty(&guard.workspace().accounts)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Show one transaction, with all of its postings, by id.")]
    fn get_transaction(
        &self,
        Parameters(p): Parameters<GetTransactionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let guard = self.lock_state();
            let found = guard
                .workspace()
                .transactions
                .iter()
                .find(|t| t.id == p.id)
                .with_context(|| format!("no transaction with id {}", p.id))?;
            Ok(serde_json::to_string_pretty(found)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Find postings, optionally filtered by account, currency and date \
        range. `account` matches either an account id or an account name. Each result \
        carries its transaction's id, date and narration.")]
    fn query_postings(
        &self,
        Parameters(p): Parameters<QueryPostingsParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let from = p.date_from.as_deref().map(parse_date).transpose()?;
            let to = p.date_to.as_deref().map(parse_date).transpose()?;
            let guard = self.lock_state();
            let ws = guard.workspace();

            // Resolve the filter to an id once, so that filtering by a
            // renamed account's *current* name still finds postings whose
            // stored name is stale.
            let wanted_id: Option<String> = match &p.account {
                None => None,
                Some(needle) => ws
                    .accounts
                    .iter()
                    .find(|a| &a.id == needle || &a.name == needle)
                    .map(|a| a.id.clone())
                    .or_else(|| Some(needle.clone())),
            };

            let mut hits: Vec<PostingHit> = Vec::new();
            for tx in &ws.transactions {
                if from.is_some_and(|f| tx.date < f) || to.is_some_and(|t| tx.date > t) {
                    continue;
                }
                for posting in &tx.postings {
                    if let Some(wanted) = &wanted_id {
                        let matches = posting.account_id.as_deref() == Some(wanted.as_str())
                            || posting.account_name.as_deref() == Some(wanted.as_str());
                        if !matches {
                            continue;
                        }
                    }
                    if p.currency.as_ref().is_some_and(|c| c != &posting.currency) {
                        continue;
                    }
                    hits.push(PostingHit {
                        transaction_id: tx.id.clone(),
                        date: tx.date,
                        narration: tx.narration.clone(),
                        account_id: posting.account_id.clone(),
                        account_name: posting.account_name.clone(),
                        amount: posting.amount.to_string(),
                        currency: posting.currency.clone(),
                    });
                }
            }
            hits.sort_by(|a, b| {
                a.date.cmp(&b.date).then_with(|| a.transaction_id.cmp(&b.transaction_id))
            });
            Ok(serde_json::to_string_pretty(&hits)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Find price-log entries, optionally filtered by base, quote and \
        date range.")]
    fn query_prices(&self, Parameters(p): Parameters<QueryPricesParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let from = p.date_from.as_deref().map(parse_date).transpose()?;
            let to = p.date_to.as_deref().map(parse_date).transpose()?;
            let guard = self.lock_state();

            let mut hits: Vec<_> = guard
                .workspace()
                .prices
                .iter()
                .filter(|e| !from.is_some_and(|f| e.date < f))
                .filter(|e| !to.is_some_and(|t| e.date > t))
                .filter(|e| !p.base.as_ref().is_some_and(|b| b != &e.base))
                .filter(|e| !p.quote.as_ref().is_some_and(|q| q != &e.quote))
                .collect();
            hits.sort_by_key(|e| e.date);
            Ok(serde_json::to_string_pretty(&hits)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Re-read the ledger from disk and report every validation issue. \
        Returns an empty array when the ledger is consistent. A posting whose stored \
        account_name is out of date is NOT an issue -- the account_id is what counts.")]
    fn validate_workspace(&self, Parameters(_): Parameters<EmptyParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let mut guard = self.lock_state();
            guard.reload()?;
            Ok(serde_json::to_string_pretty(&guard.workspace().validate())?)
        })()
        .map_err(|e| e.to_string())
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for SapphireLedgerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Sapphire Ledger is a plain-text double-entry household ledger. Use \
             list_accounts to see the chart of accounts, query_postings to find what \
             happened to an account, add_transaction to record spending, and \
             validate_workspace to check the books. Every transaction must balance: its \
             postings sum to zero per currency. An account must exist before anything \
             can be posted to it."
                .to_owned(),
        )
    }
}

// ── startup helpers ───────────────────────────────────────────────────────────

/// Open the ledger at `dir`, or find one by walking up from the working
/// directory when `dir` is None.
///
/// With `init`, the literal target (no upward search) is created and turned
/// into a ledger if it is not one already. An existing ledger is reused.
pub fn prepare_state(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<LedgerState> {
    if init {
        let target: PathBuf = match ledger_dir {
            Some(d) => d.to_path_buf(),
            None => std::env::current_dir().context("failed to read current directory")?,
        };
        if !target.join(".sapphire-ledger").exists() {
            sapphire_ledger_core::init_workspace(&target, "JPY")
                .context("failed to initialize ledger")?;
            tracing::info!("initialized sapphire-ledger in {}", target.display());
        }
        return LedgerState::open(&target).context("failed to open ledger after init");
    }

    match ledger_dir {
        Some(d) => LedgerState::open(d).with_context(|| {
            format!("not a sapphire-ledger: {} — pass --init to create one", d.display())
        }),
        None => {
            let cwd = std::env::current_dir().context("failed to read current directory")?;
            LedgerState::find(&cwd).context(
                "no sapphire-ledger found in the current directory or any parent \
                 — pass --init to create one",
            )
        }
    }
}

// ── stdio entry point ─────────────────────────────────────────────────────────

/// Serve the ledger over stdio, speaking MCP.
#[tokio::main]
pub async fn run(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<()> {
    // stdout carries JSON-RPC, so every log line goes to stderr.
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let state = prepare_state(ledger_dir, init)?;
    let server = SapphireLedgerServer::new(state);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

Replace `sapphire-ledger-mcp/src/lib.rs` entirely:

```rust
//! MCP server logic for sapphire-ledger.
//!
//! Consumed by `sapphire-ledger-cli` (stdio transport) and, later, by
//! `sapphire-ledger-server` (HTTP transport).

pub mod server;

pub use server::{run, SapphireLedgerServer};
```

- [ ] **Step 5: Run and commit**

```bash
cargo test -p sapphire-ledger-mcp
git add sapphire-ledger-mcp/Cargo.toml sapphire-ledger-mcp/src/lib.rs sapphire-ledger-mcp/src/server.rs Cargo.lock
git commit -m "feat(mcp): serve the ledger over stdio with read tools

Shaped after sapphire-journal-mcp so the HTTP transport can be added later
without moving anything."
```

Expected: 3 passed.

---

### Task 8: The MCP server — write tools

**Files:**
- Modify: `sapphire-ledger-mcp/src/server.rs`

**Interfaces:**
- Consumes: the `ops::create_*` functions (Task 5), `notify_write` (Task 7).
- Produces: MCP tools `add_account`, `add_transaction`, `add_assertion`, `add_price`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block:

```rust
    fn posting_param(account: &str, amount: &str) -> PostingParam {
        PostingParam {
            account: account.to_string(),
            amount: amount.to_string(),
            currency: "JPY".to_string(),
            price_value: None,
            price_currency: None,
            memo: None,
        }
    }

    fn add_account(server: &SapphireLedgerServer, name: &str, ty: &str) -> String {
        server
            .add_account(Parameters(AddAccountParams {
                name: name.into(),
                account_type: ty.into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect("add_account")
    }

    #[test]
    fn add_transaction_records_and_is_then_queryable() {
        let (_dir, server) = test_server();
        add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        let created = server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "イオン買い物".into(),
                payee: Some("イオン".into()),
                tags: vec!["grocery".into()],
                status: None,
                postings: vec![
                    posting_param("Expenses:Food", "1200"),
                    posting_param("Assets:Cash:JPY", "-1200"),
                ],
            }))
            .expect("add_transaction");
        assert!(created.contains("created"), "got: {created}");

        let hits = server
            .query_postings(Parameters(QueryPostingsParams {
                account: Some("Expenses:Food".into()),
                currency: None,
                date_from: None,
                date_to: None,
            }))
            .expect("query");
        assert!(hits.contains("イオン買い物"), "got: {hits}");

        let issues = server.validate_workspace(Parameters(EmptyParams {})).expect("validate");
        assert_eq!(issues.trim(), "[]", "ledger should be clean, got: {issues}");
    }

    #[test]
    fn a_posting_can_name_an_account_by_its_id() {
        let (_dir, server) = test_server();
        let food_msg = add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        // add_account reports "created account <id>: <path>"; pull the id out.
        let food_id = food_msg
            .split_whitespace()
            .nth(2)
            .expect("id in message")
            .trim_end_matches(':')
            .to_string();

        server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "by id".into(),
                payee: None,
                tags: vec![],
                status: None,
                postings: vec![
                    posting_param(&food_id, "500"),
                    posting_param("Assets:Cash:JPY", "-500"),
                ],
            }))
            .expect("add_transaction by id");

        let issues = server.validate_workspace(Parameters(EmptyParams {})).expect("validate");
        assert_eq!(issues.trim(), "[]", "got: {issues}");
    }

    #[test]
    fn add_transaction_rejects_an_unbalanced_entry() {
        let (_dir, server) = test_server();
        add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        let err = server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "wrong".into(),
                payee: None,
                tags: vec![],
                status: None,
                postings: vec![
                    posting_param("Expenses:Food", "1200"),
                    posting_param("Assets:Cash:JPY", "-999"),
                ],
            }))
            .expect_err("must reject");
        assert!(err.contains("does not balance"), "got: {err}");
    }

    #[test]
    fn add_account_rejects_an_unknown_type() {
        let (_dir, server) = test_server();
        let err = server
            .add_account(Parameters(AddAccountParams {
                name: "Assets:Cash:JPY".into(),
                account_type: "Bogus".into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect_err("must reject");
        assert!(err.contains("Bogus"), "the error should name the bad input, got: {err}");
    }
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-mcp
```

Expected: FAIL — the `Add*Params` types and the tools do not exist.

- [ ] **Step 3: Add the parameter structs and converters**

Append to the parameter-structs section of `sapphire-ledger-mcp/src/server.rs`:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddAccountParams {
    /// Colon-separated account name, e.g. `Assets:Cash:JPY`.
    pub name: String,
    /// One of: Asset, Liability, Equity, Income, Expense.
    #[serde(rename = "type")]
    pub account_type: String,
    /// Currencies this account may hold. Empty means any.
    #[serde(default)]
    pub currencies: Vec<String>,
    /// `YYYY-MM-DD` the account opened.
    pub opened_at: String,
    pub description: Option<String>,
}

/// One side of a transaction. Amounts are strings so decimals survive JSON.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PostingParam {
    /// Account id or account name — either is accepted, and the account must
    /// already exist. The stored record always keeps the id.
    pub account: String,
    /// Signed decimal, e.g. `1200` or `-1200`.
    pub amount: String,
    /// Currency code, e.g. `JPY`.
    pub currency: String,
    /// For a cross-currency posting: the unit price in `price_currency`.
    pub price_value: Option<String>,
    /// Currency the `price_value` is denominated in.
    pub price_currency: Option<String>,
    pub memo: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTransactionParams {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// What the transaction was.
    pub narration: String,
    pub payee: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `cleared` or `pending`.
    pub status: Option<String>,
    /// At least two postings, summing to zero per currency.
    pub postings: Vec<PostingParam>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BalanceParam {
    /// Decimal amount as a string.
    pub amount: String,
    pub currency: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddAssertionParams {
    /// Account id or account name.
    pub account: String,
    /// `YYYY-MM-DD`. The balance is asserted at the END of this date.
    pub date: String,
    pub balances: Vec<BalanceParam>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddPriceParams {
    pub date: String,
    /// Base currency, e.g. `USD`.
    pub base: String,
    /// Quote currency, e.g. `JPY`.
    pub quote: String,
    /// How many `quote` units one `base` unit costs, as a decimal string.
    pub rate: String,
    /// Where the rate came from, e.g. `manual`.
    pub source: Option<String>,
}

fn parse_decimal(raw: &str) -> anyhow::Result<rust_decimal::Decimal> {
    raw.parse::<rust_decimal::Decimal>()
        .with_context(|| format!("not a decimal: {raw}"))
}

fn parse_account_type(raw: &str) -> anyhow::Result<sapphire_ledger_core::AccountType> {
    use sapphire_ledger_core::AccountType::*;
    Ok(match raw {
        "Asset" => Asset,
        "Liability" => Liability,
        "Equity" => Equity,
        "Income" => Income,
        "Expense" => Expense,
        other => anyhow::bail!(
            "unknown account type {other:?}; expected one of \
             Asset, Liability, Equity, Income, Expense"
        ),
    })
}

fn parse_status(raw: &str) -> anyhow::Result<sapphire_ledger_core::TransactionStatus> {
    use sapphire_ledger_core::TransactionStatus::*;
    Ok(match raw {
        "cleared" => Cleared,
        "pending" => Pending,
        other => anyhow::bail!("unknown status {other:?}; expected `cleared` or `pending`"),
    })
}

/// Split a caller-supplied account reference into the id/name pair `ops`
/// expects. We do not know which one it is, so both are offered and `ops`
/// resolves: an exact id match wins, otherwise the name is tried.
fn split_account_ref(needle: &str, accounts: &[sapphire_ledger_core::Account])
    -> (Option<String>, Option<String>)
{
    if accounts.iter().any(|a| a.id == needle) {
        (Some(needle.to_string()), None)
    } else {
        (None, Some(needle.to_string()))
    }
}

fn build_posting(
    p: PostingParam,
    accounts: &[sapphire_ledger_core::Account],
) -> anyhow::Result<sapphire_ledger_core::Posting> {
    let price = match (p.price_value, p.price_currency) {
        (Some(value), Some(currency)) => Some(sapphire_ledger_core::Price {
            value: parse_decimal(&value)?,
            currency,
        }),
        (None, None) => None,
        _ => anyhow::bail!("price_value and price_currency must be given together"),
    };
    let (account_id, account_name) = split_account_ref(&p.account, accounts);
    Ok(sapphire_ledger_core::Posting {
        account_id,
        account_name,
        amount: parse_decimal(&p.amount)?,
        currency: p.currency,
        price,
        memo: p.memo,
    })
}
```

- [ ] **Step 4: Add the write tools**

Append inside the `#[tool_router] impl SapphireLedgerServer` block, after `validate_workspace`:

```rust
    #[tool(description = "Create an account. Fails if one with that name already exists. \
        Every account a posting names must be created first. Returns the new account's id.")]
    fn add_account(&self, Parameters(p): Parameters<AddAccountParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let account_type = parse_account_type(&p.account_type)?;
            let opened_at = parse_date(&p.opened_at)?;

            let mut guard = self.lock_state();
            let (id, dest) = ops::create_account(
                guard.root(),
                p.name,
                account_type,
                p.currencies,
                opened_at,
                p.description,
            )?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created account {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Record a transaction. Postings must sum to zero per currency, and \
        every account named must already exist. Nothing is written if validation fails.")]
    fn add_transaction(
        &self,
        Parameters(p): Parameters<AddTransactionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let status = p.status.as_deref().map(parse_status).transpose()?;

            let mut guard = self.lock_state();
            let postings = {
                let accounts = &guard.workspace().accounts;
                p.postings
                    .into_iter()
                    .map(|param| build_posting(param, accounts))
                    .collect::<anyhow::Result<Vec<_>>>()?
            };
            let (id, dest) = ops::create_transaction(
                guard.root(),
                date,
                p.narration,
                p.payee,
                p.tags,
                status,
                postings,
            )?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created transaction {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Record a balance assertion: what an account should hold at the END \
        of a date. A mismatch is a hard error when the books are checked.")]
    fn add_assertion(
        &self,
        Parameters(p): Parameters<AddAssertionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let balances = p
                .balances
                .into_iter()
                .map(|b| {
                    Ok(sapphire_ledger_core::Balance {
                        amount: parse_decimal(&b.amount)?,
                        currency: b.currency,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;

            let mut guard = self.lock_state();
            let (account_id, account_name) =
                split_account_ref(&p.account, &guard.workspace().accounts);
            let (id, dest) =
                ops::create_assertion(guard.root(), account_id, account_name, date, balances)?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created assertion {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Record an observed exchange rate in the price log: one unit of \
        `base` costs `rate` units of `quote` on `date`.")]
    fn add_price(&self, Parameters(p): Parameters<AddPriceParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let rate = parse_decimal(&p.rate)?;

            let mut guard = self.lock_state();
            let (id, dest) = ops::create_price(guard.root(), date, p.base, p.quote, rate, p.source)?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created price {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }
```

- [ ] **Step 5: Run and commit**

```bash
cargo test -p sapphire-ledger-mcp
git add sapphire-ledger-mcp/src/server.rs
git commit -m "feat(mcp): let an agent record accounts, transactions, assertions and prices

Every write goes through core::ops, so the balance check, the account
resolution and the refuse-to-overwrite rule apply identically whoever calls."
```

Expected: 7 passed. `every_tool_input_schema_declares_object_type` now covers nine tools.

---

### Task 9: Wire the CLI

**Files:**
- Modify: `sapphire-ledger-cli/src/main.rs:35-37` and its match arm, `sapphire-ledger-cli/Cargo.toml`
- Test: `sapphire-ledger-cli/tests/cli_mcp.rs`

**Interfaces:**
- Consumes: `sapphire_ledger_mcp::run` (Task 7).
- Produces: `sapphire-ledger mcp [--init]`.

- [ ] **Step 1: Adjust the manifest**

`sapphire-ledger-cli` already depends on `sapphire-ledger-mcp`. Change both path deps to opt out of default features so the store choice is forwarded explicitly:

```toml
sapphire-ledger-core = { path = "../sapphire-ledger-core", version = "0.1.0", default-features = false }
sapphire-ledger-mcp  = { path = "../sapphire-ledger-mcp",  version = "0.1.0", default-features = false }
```

then add:

```toml
[features]
default = ["redb-store"]
redb-store = ["sapphire-ledger-core/redb-store", "sapphire-ledger-mcp/redb-store"]

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write the failing test**

Create `sapphire-ledger-cli/tests/cli_mcp.rs`:

```rust
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sapphire-ledger")
}

#[test]
fn mcp_help_lists_the_init_flag() {
    let out = Command::new(bin()).args(["mcp", "--help"]).output().expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--init"), "mcp --help should document --init, got: {text}");
}

#[test]
fn check_reports_a_clean_empty_ledger() {
    let dir = tempfile::tempdir().expect("tempdir");
    let init = Command::new(bin()).arg("init").arg(dir.path()).output().expect("run init");
    assert!(init.status.success(), "init failed: {}", String::from_utf8_lossy(&init.stderr));

    let out = Command::new(bin())
        .arg("--ledger-dir")
        .arg(dir.path())
        .arg("check")
        .output()
        .expect("run check");
    assert!(out.status.success(), "check failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
}
```

- [ ] **Step 3: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-cli
```

Expected: `mcp_help_lists_the_init_flag` FAILS — there is no `--init` flag.

- [ ] **Step 4: Wire the subcommand**

In `sapphire-ledger-cli/src/main.rs`, replace the `Mcp` variant:

```rust
    /// Run the MCP server over stdio
    Mcp {
        /// Create the ledger if the target directory is not one yet
        #[arg(long)]
        init: bool,
    },
```

and its match arm:

```rust
        Command::Mcp { init } => sapphire_ledger_mcp::run(cli.ledger_dir.as_deref(), init),
```

`run` returns `anyhow::Result<()>`, matching `main`'s return type.

- [ ] **Step 5: Verify the whole workspace**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p sapphire-ledger-cli -- mcp --help
```

Expected: all tests pass, no clippy warnings, and help text showing `--init` plus the global `--ledger-dir`.

- [ ] **Step 6: Commit**

```bash
git add sapphire-ledger-cli/Cargo.toml sapphire-ledger-cli/src/main.rs sapphire-ledger-cli/tests/cli_mcp.rs Cargo.lock
git commit -m "feat(cli): hand \`sapphire-ledger mcp\` to the MCP library

Replaces the not-yet-implemented bail. --init mirrors the journal CLI so an
agent can be pointed at a directory that is not a ledger yet."
```

---

### Task 10: Update the design doc and the README

The docs describe a project that no longer matches the code. Fixing them is part of this work, not a follow-up.

**Files:**
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Correct the id terminology and document the scheme**

In `docs/design.md`, replace every occurrence of "caretta-id" with "grain-id", and add after the first mention:

```markdown
Ids come from [`grain-id`](https://crates.io/crates/grain-id). Records whose
id is their filename — transactions, assertions, prices — use
`GrainId::now_unix()`, whose decisecond resolution makes a `{year}/{MM}/`
listing time-ordered; `core::ops` re-mints on collision rather than
overwriting. **Accounts use `GrainId::random()` instead**: an account's id is
not in its path, so it does no ordering work, and a chart of accounts created
in one sitting would otherwise share a long leading prefix — the opposite of
what CLI completion wants.
```

- [ ] **Step 2: Document account identity and references**

In the "Data model" section, add before the Account subsection:

```markdown
### Account references

`Account` carries an `id`. Postings and assertions reference an account by
`account_id`, with a denormalized `account_name` beside it:

```toml
[[postings]]
account_id   = "0a1b2c3"        # authoritative; survives a rename
account_name = "Expenses:Food"  # for whoever reads the raw file
```

The id is what links; the name is never used for matching when the id is
present. This is what makes renaming an account cheap: the account file
changes and nothing else has to.

The alternative — name-only references plus a rename operation that rewrites
every transaction — was rejected because that rewrite spans many files while
remote sync resolves conflicts per path, last-writer-wins. A client adding a
transaction under the old name mid-rename would leave a record pointing at
nothing.

Three rules follow:

1. **A stale `account_name` is not an error.** Older records keep the old name
   until rewritten for some other reason. Flagging it would reintroduce the
   whole-history rewrite this design avoids.
2. **Duplicate account ids *are* an error.** An account's path comes from its
   name, so a rename is a file move; if that races under sync, one id can land
   at two paths.
3. **Either field alone is accepted on read.** Both present means the id wins;
   name-only is resolved to an id when the record is written; neither is an
   error. Hand-written TOML must not require an id lookup first.
```

- [ ] **Step 3: Rewrite the "Cache strategy" section**

Replace the whole section body with:

```markdown
There is deliberately **no ledger-specific cache**. `load_workspace` walks the
TOML files on every load, which is milliseconds at household scale.

The framework supplies mtime tracking (`sapphire-framework-track`) and search
(`sapphire-framework-retrieve`) when a consumer needs them; neither has one
yet. A ledger-specific index — postings by account, running balances — is
deferred until walking is measurably slow. See issue #1.

`rusqlite` is **not** a dependency of this workspace and should not become
one: `grain-id` carries an optional rusqlite that Cargo resolves for
`links = "sqlite3"` uniqueness even when the feature is off, so a second
rusqlite major here is a build failure rather than a design choice.
```

- [ ] **Step 4: Correct the crate structure and MCP sections**

In "Crate structure", add to the tree:

```
├── sapphire-ledger-server/    # self-hosted /rpc sync + /mcp (planned)
```

In the "MCP server" section, replace the reference to sapphire-journal#229 with:

```markdown
Modelled on `sapphire-journal-mcp` as it stands today: a server struct holding
an `Arc<Mutex<LedgerState>>`, tools declared with rmcp's `#[tool]` macro, and
a stdio entry point. The HTTP transport arrives with `sapphire-ledger-server`,
which puts `/rpc` and `/mcp` behind one set of API keys — not with the desktop
GUI, as this document originally planned.
```

- [ ] **Step 5: Update both Status sections**

In `docs/design.md`:

```markdown
- ✅ Workspace scaffold, `cargo build` clean.
- ✅ Data model (accounts, transactions, postings, prices, assertions, config).
- ✅ TOML round-trip with serde.
- ✅ Path conventions, workspace discovery, `init_workspace`.
- ✅ Repository I/O and cross-record validation + `sapphire-ledger check`.
- ✅ Record ids everywhere, with id-linked account references.
- ✅ Price-log records (storage; conversion and reporting deferred).
- ✅ `core::ops` write path.
- ✅ `LedgerState` on `sapphire-framework`.
- ✅ MCP server over stdio: read and write tools, via `sapphire-ledger mcp`.
- 🚧 `sapphire-ledger-server` (`/rpc` + `/mcp`).
- 🚧 CLI write commands.
- 🚧 Desktop GUI.
- 🚧 Price conversion / base-currency reporting.
- 🚧 Phase 2: see the issue tracker.
```

In `README.md`, replace "Early scaffolding. Data model, parser, and tools are not yet implemented." with:

```markdown
The data model, validation, write path and MCP server work. You can point an
MCP client at `sapphire-ledger mcp` and have it read and record entries. The
desktop GUI is still a scaffold, and there are no CLI write commands yet —
records are created through the MCP tools or by hand.
```

and add `sapphire-ledger-server/` to the project-structure tree with the comment `# self-hosted sync + MCP server (planned)`.

- [ ] **Step 6: Verify and commit**

```bash
grep -rn "caretta" docs/ README.md
grep -rn "rusqlite\|cache.sqlite" docs/design.md
```

Expected: the first returns nothing; the second returns only the Step 3 lines explaining why rusqlite is absent.

```bash
git add docs/design.md README.md
git commit -m "docs: describe the ledger that now exists

The cache strategy, the MCP template, the crate list, the id scheme and both
status sections all described a plan rather than the code."
```

---

## Self-Review

**Spec coverage.** Spec step 1 (`core::ops` + grain-id + the Price split + `Account.id` + the posting reference pair) is Tasks 1-5; step 2 (framework migration, dependency and state object only) is Task 6; step 3 (MCP read and write tools) is Tasks 7-8, with the CLI entry point in Task 9. The spec's tool list is fully covered: `list_accounts`, `get_transaction`, `query_postings`, `validate_workspace`, `query_prices` in Task 7, and `add_transaction`, `add_account`, `add_assertion` in Task 8, which also adds `add_price` because the spec puts price *storage* in phase 1 and a write-only-by-hand record type would be inconsistent with every other kind. The "Record identity" and "Postings reference accounts by id" sections map to Tasks 1, 3 and 5, and their three rules are each covered by a test in Task 3. Steps 4-5 of phase 1 (`sapphire-ledger-server`, agent wiring) are correctly absent. Task 10 has no matching spec step; it exists because the spec says `docs/design.md` "should be updated once this lands".

**Deliberate omissions.** `sapphire-framework-track` is declared in the workspace manifest but wired into no crate — the spec's scope note says to introduce the dependency without building indexing, and there is nothing to track yet. The `http-server` feature is absent throughout, per the Global Constraints. No `rename_account` operation exists: with id-linked references, renaming is an edit of the account record plus a file move, which the CLI/GUI work in a later phase can do directly.

**Type consistency.** `Account.id`, `Posting.account_id` / `.account_name` and `Assertion.account_id` / `.account_name` are introduced in Task 3 and used with those exact names in Tasks 5, 7 and 8. `resolve_account` takes `(Option<&str>, Option<&str>, &HashMap, &HashMap)` in Task 3 and is called that way in both Task 3's validator and Task 5's `resolve_postings` and `create_assertion`. Every `ops::create_*` returns `(String, PathBuf)` and is destructured as `(id, dest)` in Task 8 — including `create_account`, which changed from Task 5's earlier draft shape to mint the id itself. `LedgerState::open` / `find` / `workspace` / `root` / `reload` are defined in Task 6 and used in Tasks 7 and 8. `split_account_ref` and `build_posting` are defined in Task 8 before their first use; `parse_date` is defined in Task 7 and used by Task 8's tools.

**Test-behaviour consistency.** Task 3's `an_unknown_account_id_is_an_error` matches on "undefined account", which the message `undefined account id nosuch` contains. Task 8's `a_posting_can_name_an_account_by_its_id` parses the id out of `add_account`'s return string, whose format (`created account {id}: {path}`) is fixed in the same task. Task 7's `validate_workspace_reports_an_undefined_account` writes its fixture directly to disk rather than through `ops`, because `ops::create_transaction` would reject it — which is deliberate: that test covers the hand-edited-file path.

**Verified assumptions.** `AppContext::new` is `const` (`crates/sapphire-framework-workspace/src/context.rs:56`, documented as "`const` so it can be used in `static` initialisers"), so Task 6's `LEDGER_CTX` static compiles. `sapphire-ledger-cli` already depends on `sapphire-ledger-mcp`, so Task 9 adjusts that entry rather than adding it. `Cargo.toml:23`'s `rusqlite` is referenced by no member crate, which Task 1 Step 1 re-checks before deleting it.

**Known risk.** Task 6 is the first build against the framework's git dependency, which tracks `branch = "main"` and is not pinned. If it fails to build, check what `sapphire-journal-core` currently pins before assuming the plan is wrong.
