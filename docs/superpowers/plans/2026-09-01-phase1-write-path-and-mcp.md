# Phase 1 (steps 1-3): Write Path and MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give sapphire-ledger a `core::ops` write API with generated ids, and an MCP server exposing read and write tools over stdio, so an AI agent can record and query transactions.

**Architecture:** All writes funnel through one `core::ops` module that mints a `grain-id`, resolves the canonical `{kind}/{year}/{MM}/{id}.toml` path, validates, and refuses to overwrite. `sapphire-ledger-mcp` becomes a library exposing an rmcp `ServerHandler` over stdio, holding a `LedgerState` behind an `Arc<Mutex<_>>`, modelled directly on `sapphire-journal-mcp`. The dead `rusqlite` pin is dropped and the framework dependency added, but no index is built.

**Tech Stack:** Rust 2024 edition, `rmcp` 1.5, `schemars` 1.0, `grain-id`, `rust_decimal`, `chrono`, `toml` 1.1, `sapphire-framework-workspace`, `sapphire-framework-track`.

**Spec:** [`docs/superpowers/specs/2026-09-01-restart-roadmap-design.md`](../specs/2026-09-01-restart-roadmap-design.md)

## Global Constraints

- **Rust edition 2024**, workspace resolver `3`. All member crates use `edition.workspace = true`.
- **Licensing:** every crate stays `MIT OR Apache-2.0` (`license.workspace = true`).
- **Decimals are persisted as TOML strings**, always via `#[serde(with = "rust_decimal::serde::str")]`. TOML has no decimal type.
- **One record, one file.** Never write two records into one file.
- **Record paths** are `{kind}/{year}/{MM}/{id}.toml` for transactions, assertions and prices; `accounts/{Segment}/.../{Leaf}.toml` for accounts.
- **Ids are `grain-id`**, rendered as the 7-character `Display` form. Generated with `GrainId::now_unix()` (decisecond precision), never `GrainId::random()` — the design depends on ids being time-ordered.
- **Writes never overwrite.** Every create refuses if the destination file exists.
- **`Workspace::validate()` never short-circuits** — it returns every issue found.
- **The MCP server logs to stderr only.** stdout carries JSON-RPC and must stay clean.
- **Do not add `rusqlite`** to any crate in this workspace. Do not build a SQLite cache.
- **Do not add the `http-server` feature or the server crate** in this plan — those are phase 1 step 4, out of scope here.

---

## File Structure

**`sapphire-ledger-core/src/`**
- `prices.rs` — **new.** `PriceEntry` record type and its relative-path helper. Also becomes the home of the inline `Price` type moved out of `transaction.rs`.
- `ops.rs` — **new.** The single write path: `new_id`, `create_account`, `create_transaction`, `create_assertion`, `create_price`.
- `state.rs` — **new.** `LedgerState`: an `AppContext` plus a loaded `Workspace`, with `reload()`.
- `transaction.rs` — modified: `Price` moves out; re-exported for compatibility within the crate.
- `workspace.rs` — modified: `PRICES_DIR`, `price_relative_path`, `init_workspace` creates `prices/`.
- `repository.rs` — modified: `Workspace` gains `prices`; `load_workspace` loads them.
- `validate.rs` — modified: price entries validated against known currencies.
- `lib.rs` — modified: module list and re-exports.

**`sapphire-ledger-mcp/src/`**
- `lib.rs` — modified from a 5-line stub: declares `server`, re-exports `run` and `SapphireLedgerServer`.
- `server.rs` — **new.** Parameter structs, the `#[tool_router]` impl, `ServerHandler`, `prepare_state`, and the stdio `run` entry point.

**`sapphire-ledger-cli/src/`**
- `main.rs` — modified: `Command::Mcp` gains `--init` and calls into the library.

**Manifests**
- `Cargo.toml` — remove `rusqlite`; add `grain-id`, framework deps, `tempfile` dev-dep.
- `sapphire-ledger-core/Cargo.toml`, `sapphire-ledger-mcp/Cargo.toml`, `sapphire-ledger-cli/Cargo.toml` — dependency and feature wiring.

---

### Task 1: Drop the dead rusqlite pin and add grain-id

The workspace manifest declares `rusqlite = { version = "0.39", features = ["bundled"] }` at `Cargo.toml:23`, and **no member crate references it**. `grain-id` needs `rusqlite 0.40.2` behind an optional feature that Cargo still resolves for `links = "sqlite3"` uniqueness, so the stale pin must go before grain-id lands.

**Files:**
- Modify: `Cargo.toml:23`
- Modify: `sapphire-ledger-core/Cargo.toml`
- Test: `sapphire-ledger-core/src/ops.rs` (inline `#[cfg(test)]` module)

**Interfaces:**
- Consumes: nothing.
- Produces: `sapphire_ledger_core::ops::new_id() -> String`.

- [ ] **Step 1: Confirm the pin is genuinely unused**

```bash
grep -rn "rusqlite" sapphire-ledger-*/src sapphire-ledger-*/Cargo.toml
```

Expected: no output. If anything matches, stop — the premise of this task is wrong and the plan needs revisiting.

- [ ] **Step 2: Remove the rusqlite line and add grain-id**

In `Cargo.toml`, delete the `rusqlite = ...` line from `[workspace.dependencies]` and add:

```toml
grain-id = { version = "0.15", features = ["serde", "schemars"] }
tempfile = "3"
```

Do **not** enable grain-id's `rusqlite` feature. Ledger needs id generation only.

- [ ] **Step 3: Add grain-id to core**

In `sapphire-ledger-core/Cargo.toml`, under `[dependencies]`:

```toml
grain-id.workspace = true
```

and under a `[dev-dependencies]` section (create it if absent):

```toml
tempfile.workspace = true
```

- [ ] **Step 4: Write the failing test**

Create `sapphire-ledger-core/src/ops.rs`:

```rust
//! The single write path for every record kind.
//!
//! CLI, MCP, GUI and importers all funnel through here so that id
//! generation, path resolution, validation and the refuse-to-overwrite
//! rule exist in exactly one place.

use grain_id::GrainId;

/// Mint a new record id.
///
/// Uses `GrainId::now_unix()` (decisecond precision) rather than
/// `GrainId::random()` so ids sort by creation time. Two records minted
/// inside the same decisecond collide; callers resolve that by retrying,
/// which the create functions do.
pub fn new_id() -> String {
    GrainId::now_unix().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_is_seven_chars() {
        let id = new_id();
        assert_eq!(id.chars().count(), 7, "grain-id renders as 7 characters, got {id:?}");
    }

    #[test]
    fn new_id_is_not_nil() {
        assert_ne!(new_id(), "0000000");
    }
}
```

Add to `sapphire-ledger-core/src/lib.rs` after the existing `pub mod` lines:

```rust
pub mod ops;
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-core ops::
```

Expected: 2 passed. If the build fails on a `libsqlite3-sys` `links` collision, the rusqlite pin was not fully removed.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock sapphire-ledger-core/Cargo.toml sapphire-ledger-core/src/ops.rs sapphire-ledger-core/src/lib.rs
git commit -m "feat(core): mint record ids with grain-id, and drop the dead rusqlite pin

The workspace declared rusqlite 0.39 that no member crate used. grain-id
wants 0.40.2 behind an optional feature Cargo still resolves for links
uniqueness, so the stale pin had to go first."
```

---

### Task 2: Split Price from PriceEntry

`Price` (the inline per-posting price) is publicly re-exported at `sapphire-ledger-core/src/lib.rs:24`. Once `schemars` generates MCP tool schemas from these types the name is published, so the split happens now, before Task 6 exposes anything.

**Files:**
- Create: `sapphire-ledger-core/src/prices.rs`
- Modify: `sapphire-ledger-core/src/transaction.rs:16-21`
- Modify: `sapphire-ledger-core/src/lib.rs`
- Test: `sapphire-ledger-core/src/prices.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing.
- Produces: `prices::Price { value: Decimal, currency: String }`, `prices::PriceEntry { id: String, date: NaiveDate, base: String, quote: String, rate: Decimal, source: Option<String>, created_at: DateTime<FixedOffset>, updated_at: DateTime<FixedOffset> }`.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-core/src/prices.rs`:

```rust
//! Exchange rates, in two unrelated shapes.
//!
//! [`Price`] is the inline price carried by a single posting, used to make a
//! cross-currency transaction balance. [`PriceEntry`] is a standalone record
//! in the price log, used to report in a base currency at a past date. They
//! are deliberately separate types: they share a domain but not a lifecycle,
//! and one of them is part of the published MCP tool schema.

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
        assert_eq!(entry.base, "USD");
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
        assert!(!rendered.contains("source"), "absent source must not be written: {rendered}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-core prices::
```

Expected: FAIL — `prices` is not a declared module.

- [ ] **Step 3: Wire the module and move the inline type**

In `sapphire-ledger-core/src/lib.rs`, add `pub mod prices;` to the module list.

In `sapphire-ledger-core/src/transaction.rs`, **delete** the `Price` struct definition (lines 16-21) and replace the removed block with a re-export so the rest of the file is untouched:

```rust
pub use crate::prices::Price;
```

- [ ] **Step 4: Update the crate's public re-exports**

In `sapphire-ledger-core/src/lib.rs`, change the transaction re-export line and add the prices one:

```rust
pub use prices::{Price, PriceEntry};
pub use transaction::{Posting, Transaction, TransactionStatus};
```

`Price` is now re-exported from exactly one place. Leaving it in both lists is a duplicate-import compile error.

- [ ] **Step 5: Run the full core test suite**

```bash
cargo test -p sapphire-ledger-core
```

Expected: all pass, including the two new price tests.

- [ ] **Step 6: Commit**

```bash
git add sapphire-ledger-core/src/prices.rs sapphire-ledger-core/src/transaction.rs sapphire-ledger-core/src/lib.rs
git commit -m "refactor(core): separate the inline posting price from the price-log record

They share a domain but not a lifecycle, and Price is about to become part
of the published MCP tool schema -- renaming it after that ships would be a
schema break."
```

---

### Task 3: Price paths and workspace loading

**Files:**
- Modify: `sapphire-ledger-core/src/workspace.rs:11-17` (constants), and `init_workspace`
- Modify: `sapphire-ledger-core/src/repository.rs` (`Workspace`, `load_workspace`)
- Modify: `sapphire-ledger-core/src/lib.rs` (re-exports)
- Test: `sapphire-ledger-core/tests/workspace_prices.rs`

**Interfaces:**
- Consumes: `prices::PriceEntry` (Task 2).
- Produces: `workspace::PRICES_DIR: &str`, `workspace::price_relative_path(date: NaiveDate, id: &str) -> PathBuf`, and a `prices: Vec<PriceEntry>` field on `Workspace`.

- [ ] **Step 1: Write the failing test**

Create `sapphire-ledger-core/tests/workspace_prices.rs`:

```rust
use sapphire_ledger_core::{init_workspace, load_workspace, price_relative_path, save_toml, PriceEntry};

#[test]
fn init_creates_the_prices_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_workspace(dir.path(), "JPY").expect("init");
    assert!(dir.path().join("prices").is_dir(), "init_workspace must create prices/");
}

#[test]
fn price_path_is_year_month_id() {
    let date = "2026-05-21".parse().unwrap();
    let path = price_relative_path(date, "0a1b2c3");
    assert_eq!(path, std::path::Path::new("prices/2026/05/0a1b2c3.toml"));
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
    let dest = dir.path().join(price_relative_path(entry.date, &entry.id));
    save_toml(&dest, &entry).expect("save");

    let ws = load_workspace(dir.path()).expect("load");
    assert_eq!(ws.prices.len(), 1);
    assert_eq!(ws.prices[0].base, "USD");
}
```

- [ ] **Step 2: Run it to confirm it fails**

```bash
cargo test -p sapphire-ledger-core --test workspace_prices
```

Expected: FAIL — `price_relative_path` and `PriceEntry` are not exported, `Workspace` has no `prices`.

- [ ] **Step 3: Add the constant and path helper**

In `sapphire-ledger-core/src/workspace.rs`, beside `ASSERTIONS_DIR`:

```rust
pub const PRICES_DIR: &str = "prices";
```

and beside `assertion_relative_path`:

```rust
/// Relative path for a price-log entry: `prices/{year}/{MM}/{id}.toml`.
pub fn price_relative_path(date: NaiveDate, id: &str) -> PathBuf {
    PathBuf::from(PRICES_DIR)
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{id}.{TOML_EXTENSION}"))
}
```

In `init_workspace`, beside the other `create_dir_all` calls:

```rust
    fs::create_dir_all(target.join(PRICES_DIR))?;
```

- [ ] **Step 4: Load prices into the workspace**

In `sapphire-ledger-core/src/repository.rs`, add `use crate::prices::PriceEntry;`, add `PRICES_DIR` to the `crate::workspace::{...}` import list, add the field to `Workspace`:

```rust
    pub prices: Vec<PriceEntry>,
```

and in `load_workspace`, before the `Ok(Workspace { ... })`:

```rust
    let prices = walk_toml_files(&root.join(PRICES_DIR))?
        .iter()
        .map(|p| load_toml::<PriceEntry>(p))
        .collect::<Result<Vec<_>>>()?;
```

then add `prices,` to the struct literal.

- [ ] **Step 5: Export the new items**

In `sapphire-ledger-core/src/lib.rs`, add `price_relative_path` and `PRICES_DIR` to the `pub use workspace::{...}` list.

- [ ] **Step 6: Run the tests**

```bash
cargo test -p sapphire-ledger-core
```

Expected: all pass, including the three new tests.

- [ ] **Step 7: Commit**

```bash
git add sapphire-ledger-core/src/workspace.rs sapphire-ledger-core/src/repository.rs sapphire-ledger-core/src/lib.rs sapphire-ledger-core/tests/workspace_prices.rs
git commit -m "feat(core): store and load price-log entries"
```

---

### Task 4: The create functions

Every create mints an id, resolves the canonical path, validates, and refuses to overwrite. Because `GrainId::now_unix()` has decisecond precision, two records created in the same decisecond collide; the create functions retry rather than failing.

**Files:**
- Modify: `sapphire-ledger-core/src/ops.rs`
- Test: `sapphire-ledger-core/tests/ops_create.rs`

**Interfaces:**
- Consumes: `new_id` (Task 1), `PriceEntry` (Task 2), `price_relative_path` (Task 3).
- Produces:
  - `ops::create_account(root: &Path, account: &Account) -> Result<PathBuf>`
  - `ops::create_transaction(root: &Path, date: NaiveDate, narration: String, payee: Option<String>, tags: Vec<String>, status: Option<TransactionStatus>, postings: Vec<Posting>) -> Result<(String, PathBuf)>`
  - `ops::create_assertion(root: &Path, account: String, date: NaiveDate, balances: Vec<Balance>) -> Result<(String, PathBuf)>`
  - `ops::create_price(root: &Path, date: NaiveDate, base: String, quote: String, rate: Decimal, source: Option<String>) -> Result<(String, PathBuf)>`

- [ ] **Step 1: Write the failing tests**

Create `sapphire-ledger-core/tests/ops_create.rs`:

```rust
use rust_decimal::Decimal;
use sapphire_ledger_core::{
    ops, Account, AccountType, Balance, Posting, TransactionStatus,
};

fn ws() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");
    dir
}

fn account(name: &str, account_type: AccountType) -> Account {
    Account {
        name: name.to_string(),
        account_type,
        currencies: vec![],
        opened_at: "2026-01-01".parse().unwrap(),
        closed_at: None,
        description: None,
    }
}

fn posting(acct: &str, amount: &str) -> Posting {
    Posting {
        account: acct.to_string(),
        amount: amount.parse::<Decimal>().unwrap(),
        currency: "JPY".to_string(),
        price: None,
        memo: None,
    }
}

#[test]
fn create_account_writes_the_hierarchical_path() {
    let dir = ws();
    let dest = ops::create_account(dir.path(), &account("Assets:Cash:JPY", AccountType::Asset))
        .expect("create");
    assert!(dest.ends_with("accounts/Assets/Cash/JPY.toml"), "got {}", dest.display());
    assert!(dest.is_file());
}

#[test]
fn create_account_refuses_to_overwrite() {
    let dir = ws();
    let acct = account("Assets:Cash:JPY", AccountType::Asset);
    ops::create_account(dir.path(), &acct).expect("first create");
    let err = ops::create_account(dir.path(), &acct).expect_err("second create must fail");
    assert!(err.to_string().contains("already exists"), "got: {err}");
}

#[test]
fn create_account_rejects_a_malformed_name() {
    let dir = ws();
    let err = ops::create_account(dir.path(), &account("Assets::Cash", AccountType::Asset))
        .expect_err("empty segment must be rejected");
    assert!(err.to_string().contains("empty segment"), "got: {err}");
}

#[test]
fn create_transaction_returns_an_id_and_a_dated_path() {
    let dir = ws();
    let (id, dest) = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "イオン買い物".into(),
        Some("イオン".into()),
        vec!["grocery".into()],
        Some(TransactionStatus::Cleared),
        vec![posting("Expenses:Food", "1200"), posting("Assets:Cash:JPY", "-1200")],
    )
    .expect("create");

    assert_eq!(id.chars().count(), 7);
    assert!(dest.ends_with(format!("transactions/2026/05/{id}.toml")), "got {}", dest.display());
}

#[test]
fn create_transaction_rejects_an_unbalanced_entry() {
    let dir = ws();
    let err = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "wrong".into(),
        None,
        vec![],
        None,
        vec![posting("Expenses:Food", "1200"), posting("Assets:Cash:JPY", "-999")],
    )
    .expect_err("unbalanced must be rejected");
    assert!(err.to_string().contains("does not balance"), "got: {err}");
}

#[test]
fn create_transaction_writes_nothing_when_validation_fails() {
    let dir = ws();
    let _ = ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "wrong".into(),
        None,
        vec![],
        None,
        vec![posting("Expenses:Food", "1200"), posting("Assets:Cash:JPY", "-999")],
    );
    let month = dir.path().join("transactions/2026/05");
    let count = std::fs::read_dir(&month).map(|d| d.count()).unwrap_or(0);
    assert_eq!(count, 0, "a rejected transaction must leave no file behind");
}

#[test]
fn create_assertion_and_price_produce_dated_paths() {
    let dir = ws();
    let (aid, apath) = ops::create_assertion(
        dir.path(),
        "Assets:Cash:JPY".into(),
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
fn created_records_survive_a_reload() {
    let dir = ws();
    ops::create_account(dir.path(), &account("Expenses:Food", AccountType::Expense)).unwrap();
    ops::create_account(dir.path(), &account("Assets:Cash:JPY", AccountType::Asset)).unwrap();
    ops::create_transaction(
        dir.path(),
        "2026-05-21".parse().unwrap(),
        "イオン買い物".into(),
        None,
        vec![],
        None,
        vec![posting("Expenses:Food", "1200"), posting("Assets:Cash:JPY", "-1200")],
    )
    .unwrap();

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

Expected: FAIL — none of the `create_*` functions exist.

- [ ] **Step 3: Implement the create functions**

Append to `sapphire-ledger-core/src/ops.rs` (below `new_id`, above the `#[cfg(test)]` module):

```rust
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use rust_decimal::Decimal;

use crate::account::Account;
use crate::assertion::{Assertion, Balance};
use crate::error::{Error, Result};
use crate::prices::PriceEntry;
use crate::repository::save_toml;
use crate::transaction::{Posting, Transaction, TransactionStatus};
use crate::workspace::{
    account_relative_path, assertion_relative_path, price_relative_path,
    transaction_relative_path,
};

/// How many times to re-mint an id when the destination is already taken.
///
/// `new_id` has decisecond resolution, so a collision means "another record
/// was created in this same tenth of a second". Sleeping is not needed:
/// re-minting after the clock advances is enough, and a handful of attempts
/// covers any realistic burst.
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

/// Mint an id whose destination file does not yet exist.
///
/// `relative` maps a candidate id to its workspace-relative path.
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

/// Write a new account. The path is derived from the account name, so there
/// is no id to mint — a duplicate name is a hard error.
pub fn create_account(root: &Path, account: &Account) -> Result<PathBuf> {
    let dest = root.join(account_relative_path(&account.name)?);
    refuse_existing(&dest)?;
    save_toml(&dest, account)?;
    Ok(dest)
}

/// Write a new transaction, validating it before anything touches disk.
pub fn create_transaction(
    root: &Path,
    date: NaiveDate,
    narration: String,
    payee: Option<String>,
    tags: Vec<String>,
    status: Option<TransactionStatus>,
    postings: Vec<Posting>,
) -> Result<(String, PathBuf)> {
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
    account: String,
    date: NaiveDate,
    balances: Vec<Balance>,
) -> Result<(String, PathBuf)> {
    if balances.is_empty() {
        return Err(Error::Validation(
            "assertion must declare at least one balance".into(),
        ));
    }
    let (id, dest) = mint_free_id(root, |id| assertion_relative_path(date, id))?;
    let timestamp = now();
    let assertion = Assertion {
        id: id.clone(),
        account,
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

Expected: all pass. Note `create_transaction` validates *before* `save_toml`, which is what `create_transaction_writes_nothing_when_validation_fails` checks.

- [ ] **Step 5: Commit**

```bash
git add sapphire-ledger-core/src/ops.rs sapphire-ledger-core/tests/ops_create.rs
git commit -m "feat(core): one write path for every record kind

Mints an id, resolves the canonical path, validates, refuses to overwrite.
CLI, MCP and GUI all go through here rather than reimplementing it three
times with three different sets of mistakes."
```

---

### Task 5: LedgerState and the framework dependency

Per the spec's scope note, this task introduces the state object and the dependency **only**. It builds no index: nothing in phase 1 steps 1-3 has a consumer for one.

**Files:**
- Create: `sapphire-ledger-core/src/state.rs`
- Modify: `sapphire-ledger-core/src/lib.rs`
- Modify: `sapphire-ledger-core/Cargo.toml`
- Modify: `Cargo.toml` (workspace deps)
- Test: `sapphire-ledger-core/tests/state_reload.rs`

**Interfaces:**
- Consumes: `load_workspace`, `find_workspace_root`, `ops::create_account` (Task 4).
- Produces: `state::LedgerState`, with `LedgerState::open(root: &Path) -> Result<Self>`, `LedgerState::find(start: &Path) -> Result<Self>`, `LedgerState::workspace(&self) -> &Workspace`, `LedgerState::root(&self) -> &Path`, `LedgerState::reload(&mut self) -> Result<()>`. Also `sapphire_ledger_core::LEDGER_CTX`.

- [ ] **Step 1: Add the framework dependencies**

In the workspace `Cargo.toml` under `[workspace.dependencies]`:

```toml
sapphire-workspace = { package = "sapphire-framework-workspace", git = "https://github.com/fluo10/sapphire-framework", branch = "main", default-features = false }
sapphire-track = { package = "sapphire-framework-track", git = "https://github.com/fluo10/sapphire-framework", branch = "main" }
```

The `package = ` alias keeps the short extern name, matching how `sapphire-journal-core` does it.

In `sapphire-ledger-core/Cargo.toml`, add under `[dependencies]`:

```toml
sapphire-workspace.workspace = true
```

and add a `[features]` section:

```toml
[features]
default = ["redb-store"]
redb-store = ["sapphire-workspace/redb-store"]
```

`sapphire-track` is declared at the workspace level for the later index work but is deliberately not wired into any crate yet — nothing consumes it in this plan.

- [ ] **Step 2: Write the failing test**

Create `sapphire-ledger-core/tests/state_reload.rs`:

```rust
use sapphire_ledger_core::{ops, Account, AccountType, LedgerState};

#[test]
fn reload_picks_up_a_record_written_after_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");

    let mut state = LedgerState::open(dir.path()).expect("open");
    assert_eq!(state.workspace().accounts.len(), 0);

    ops::create_account(
        dir.path(),
        &Account {
            name: "Assets:Cash:JPY".into(),
            account_type: AccountType::Asset,
            currencies: vec![],
            opened_at: "2026-01-01".parse().unwrap(),
            closed_at: None,
            description: None,
        },
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
//! [`LedgerState`] is the single object frontends (CLI, MCP, GUI) hold while
//! a ledger is active, mirroring `JournalState` in sapphire-journal.
//!
//! It currently holds an eagerly-loaded [`Workspace`] and nothing else. The
//! search and mtime-tracking infrastructure the framework offers has no
//! consumer yet: no tool in this phase searches, and the ledger-specific
//! index is deliberately deferred. This type exists now so that adding them
//! later is a change inside one struct rather than a change to every caller.

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
    /// Open the ledger rooted at `root` (the directory containing
    /// `.sapphire-ledger/`) and load every record.
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

    /// The loaded records. This is a snapshot taken at `open` or the last
    /// `reload` — a write through `ops` does not update it.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Re-read every record from disk. Callers do this after writing, and
    /// periodically to pick up edits made by git, sync, or a human editor.
    pub fn reload(&mut self) -> Result<()> {
        self.workspace = load_workspace(&self.root)?;
        Ok(())
    }
}
```

- [ ] **Step 5: Declare the module, the app context, and the re-export**

In `sapphire-ledger-core/src/lib.rs`, add `pub mod state;` to the module list, add the re-export:

```rust
pub use state::LedgerState;
```

and add the shared application context beside it:

```rust
/// Process-wide application context, naming the cache and data directories
/// the framework uses. Mirrors `JOURNAL_CTX` in sapphire-journal.
pub static LEDGER_CTX: sapphire_workspace::AppContext =
    sapphire_workspace::AppContext::new("sapphire-ledger");
```

- [ ] **Step 6: Run the tests**

```bash
cargo test -p sapphire-ledger-core
```

Expected: all pass. The first framework build pulls a git dependency and will be slow.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock sapphire-ledger-core/Cargo.toml sapphire-ledger-core/src/state.rs sapphire-ledger-core/src/lib.rs sapphire-ledger-core/tests/state_reload.rs
git commit -m "feat(core): hold an open ledger in a LedgerState, on the framework

No index yet -- nothing in this phase searches. The point is that the state
object and the framework dependency exist now, while ledger is small enough
that introducing them costs one struct."
```

---

### Task 6: The MCP server — read tools

**Files:**
- Create: `sapphire-ledger-mcp/src/server.rs`
- Modify: `sapphire-ledger-mcp/src/lib.rs`
- Modify: `sapphire-ledger-mcp/Cargo.toml`
- Test: inline `#[cfg(test)]` in `sapphire-ledger-mcp/src/server.rs`

**Interfaces:**
- Consumes: `LedgerState` (Task 5), `Workspace` fields (Task 3).
- Produces: `SapphireLedgerServer::new(state: LedgerState) -> Self`, `SapphireLedgerServer::from_shared(state: Arc<Mutex<LedgerState>>) -> Self`, `SapphireLedgerServer::shared_state(&self) -> Arc<Mutex<LedgerState>>`, and `prepare_state(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<LedgerState>`.

- [ ] **Step 1: Set up the crate manifest**

Replace the `[dependencies]` block of `sapphire-ledger-mcp/Cargo.toml` (creating sections as needed):

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

Create `sapphire-ledger-mcp/src/server.rs` containing only the test module for now:

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
        sapphire_ledger_core::ops::create_transaction(
            dir.path(),
            "2026-05-21".parse().unwrap(),
            "orphan".into(),
            None,
            vec![],
            None,
            vec![
                sapphire_ledger_core::Posting {
                    account: "Expenses:Nowhere".into(),
                    amount: "10".parse().unwrap(),
                    currency: "JPY".into(),
                    price: None,
                    memo: None,
                },
                sapphire_ledger_core::Posting {
                    account: "Assets:Nowhere".into(),
                    amount: "-10".parse().unwrap(),
                    currency: "JPY".into(),
                    price: None,
                    memo: None,
                },
            ],
        )
        .expect("create");

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
//! a stdio entry point. The HTTP transport is added later alongside
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
use sapphire_ledger_core::{LedgerState, ops};
use serde::Deserialize;

/// Called after a tool writes, with every path the write touched.
///
/// One call is one batch: a write that produces several files reports them
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
    /// spawns one server per session but all of them must see one ledger.
    pub fn from_shared(state: Arc<Mutex<LedgerState>>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
            write_observer: None,
        }
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
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    /// Exact account name, e.g. `Assets:Cash:JPY`.
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
    /// Base currency, e.g. `USD`.
    pub base: Option<String>,
    /// Quote currency, e.g. `JPY`.
    pub quote: Option<String>,
    /// Inclusive lower bound, `YYYY-MM-DD`.
    pub date_from: Option<String>,
    /// Inclusive upper bound, `YYYY-MM-DD`.
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
    account: String,
    amount: String,
    currency: String,
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl SapphireLedgerServer {
    #[tool(description = "List every account with its type, allowed currencies, and open dates.")]
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

    #[tool(description = "Find postings, optionally filtered by account, currency and date range. \
        Each result carries its transaction's id, date and narration.")]
    fn query_postings(
        &self,
        Parameters(p): Parameters<QueryPostingsParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let from = p.date_from.as_deref().map(parse_date).transpose()?;
            let to = p.date_to.as_deref().map(parse_date).transpose()?;
            let guard = self.lock_state();

            let mut hits: Vec<PostingHit> = Vec::new();
            for tx in &guard.workspace().transactions {
                if from.is_some_and(|f| tx.date < f) || to.is_some_and(|t| tx.date > t) {
                    continue;
                }
                for posting in &tx.postings {
                    if p.account.as_ref().is_some_and(|a| a != &posting.account) {
                        continue;
                    }
                    if p.currency.as_ref().is_some_and(|c| c != &posting.currency) {
                        continue;
                    }
                    hits.push(PostingHit {
                        transaction_id: tx.id.clone(),
                        date: tx.date,
                        narration: tx.narration.clone(),
                        account: posting.account.clone(),
                        amount: posting.amount.to_string(),
                        currency: posting.currency.clone(),
                    });
                }
            }
            hits.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.transaction_id.cmp(&b.transaction_id)));
            Ok(serde_json::to_string_pretty(&hits)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Find price-log entries, optionally filtered by base, quote and date range.")]
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

    #[tool(description = "Re-read the ledger from disk and report every validation issue found. \
        Returns an empty array when the ledger is consistent.")]
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
            "Sapphire Ledger is a plain-text double-entry household ledger. \
             Use list_accounts to see the chart of accounts, query_postings to \
             find what happened to an account, add_transaction to record \
             spending, and validate_workspace to check the books. Every \
             transaction must balance: its postings sum to zero per currency."
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
        Some(d) => LedgerState::open(d)
            .with_context(|| format!("not a sapphire-ledger: {} — pass --init to create one", d.display())),
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

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-mcp
```

Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add sapphire-ledger-mcp/Cargo.toml sapphire-ledger-mcp/src/lib.rs sapphire-ledger-mcp/src/server.rs Cargo.lock
git commit -m "feat(mcp): serve the ledger over stdio with read tools

list_accounts, get_transaction, query_postings, query_prices,
validate_workspace. Shaped after sapphire-journal-mcp so the HTTP transport
can be added later without moving anything."
```

---

### Task 7: The MCP server — write tools

**Files:**
- Modify: `sapphire-ledger-mcp/src/server.rs`

**Interfaces:**
- Consumes: `ops::create_account`, `ops::create_transaction`, `ops::create_assertion`, `ops::create_price` (Task 4); `notify_write` (Task 6).
- Produces: MCP tools `add_account`, `add_transaction`, `add_assertion`, `add_price`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `sapphire-ledger-mcp/src/server.rs`:

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

    #[test]
    fn add_transaction_records_and_is_then_queryable() {
        let (_dir, server) = test_server();

        server
            .add_account(Parameters(AddAccountParams {
                name: "Expenses:Food".into(),
                account_type: "Expense".into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect("expense account");
        server
            .add_account(Parameters(AddAccountParams {
                name: "Assets:Cash:JPY".into(),
                account_type: "Asset".into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect("asset account");

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

        let issues = server
            .validate_workspace(Parameters(EmptyParams {}))
            .expect("validate");
        assert_eq!(issues.trim(), "[]", "ledger should be clean, got: {issues}");
    }

    #[test]
    fn add_transaction_rejects_an_unbalanced_entry() {
        let (_dir, server) = test_server();
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
            .expect_err("unbalanced must be rejected");
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
            .expect_err("unknown type must be rejected");
        assert!(err.contains("Bogus"), "the error should name the bad input, got: {err}");
    }
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-mcp
```

Expected: FAIL — the `Add*Params` types and the tools do not exist.

- [ ] **Step 3: Add the parameter structs**

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
    /// Account name; must already exist.
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
    pub account: String,
    /// `YYYY-MM-DD`. The balance is asserted at the END of this date.
    pub date: String,
    pub balances: Vec<BalanceParam>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddPriceParams {
    /// `YYYY-MM-DD`.
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

fn build_posting(p: PostingParam) -> anyhow::Result<sapphire_ledger_core::Posting> {
    let price = match (p.price_value, p.price_currency) {
        (Some(value), Some(currency)) => Some(sapphire_ledger_core::Price {
            value: parse_decimal(&value)?,
            currency,
        }),
        (None, None) => None,
        _ => anyhow::bail!("price_value and price_currency must be given together"),
    };
    Ok(sapphire_ledger_core::Posting {
        account: p.account,
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
        Every posting's account must exist before it can be posted to.")]
    fn add_account(&self, Parameters(p): Parameters<AddAccountParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let account = sapphire_ledger_core::Account {
                name: p.name,
                account_type: parse_account_type(&p.account_type)?,
                currencies: p.currencies,
                opened_at: parse_date(&p.opened_at)?,
                closed_at: None,
                description: p.description,
            };
            let mut guard = self.lock_state();
            let dest = ops::create_account(guard.root(), &account)?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created account: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Record a transaction. Postings must sum to zero per currency, \
        and every account named must already exist. Nothing is written if validation fails.")]
    fn add_transaction(
        &self,
        Parameters(p): Parameters<AddTransactionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let status = p.status.as_deref().map(parse_status).transpose()?;
            let postings = p
                .postings
                .into_iter()
                .map(build_posting)
                .collect::<anyhow::Result<Vec<_>>>()?;

            let mut guard = self.lock_state();
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

    #[tool(description = "Record a balance assertion: what an account should hold at the \
        END of a date. A mismatch is a hard error when the books are checked.")]
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
            let (id, dest) = ops::create_assertion(guard.root(), p.account, date, balances)?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created assertion {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Record an observed exchange rate in the price log: \
        one unit of `base` costs `rate` units of `quote` on `date`.")]
    fn add_price(&self, Parameters(p): Parameters<AddPriceParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let rate = parse_decimal(&p.rate)?;

            let mut guard = self.lock_state();
            let (id, dest) =
                ops::create_price(guard.root(), date, p.base, p.quote, rate, p.source)?;
            guard.reload()?;
            drop(guard);
            self.notify_write(&[dest.clone()]);
            Ok(format!("created price {id}: {}", dest.display()))
        })()
        .map_err(|e| e.to_string())
    }
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-mcp
```

Expected: 6 passed. `every_tool_input_schema_declares_object_type` now covers nine tools.

- [ ] **Step 6: Commit**

```bash
git add sapphire-ledger-mcp/src/server.rs
git commit -m "feat(mcp): let an agent record accounts, transactions, assertions and prices

Every write goes through core::ops, so the balance check and the
refuse-to-overwrite rule apply identically whoever is calling."
```

---

### Task 8: Wire the CLI and close the loop

**Files:**
- Modify: `sapphire-ledger-cli/src/main.rs:35-37` (the `Mcp` variant) and `:74-76` (its match arm)
- Modify: `sapphire-ledger-cli/Cargo.toml`
- Test: `sapphire-ledger-cli/tests/cli_mcp.rs`

**Interfaces:**
- Consumes: `sapphire_ledger_mcp::run` (Task 6).
- Produces: the `sapphire-ledger mcp [--init]` subcommand.

- [ ] **Step 1: Adjust the dependency**

`sapphire-ledger-cli` already depends on `sapphire-ledger-mcp`. Change that line to opt out of default features so the store choice is forwarded explicitly:

```toml
sapphire-ledger-mcp  = { path = "../sapphire-ledger-mcp",  version = "0.1.0", default-features = false }
```

and do the same for its `sapphire-ledger-core` line:

```toml
sapphire-ledger-core = { path = "../sapphire-ledger-core", version = "0.1.0", default-features = false }
```

then add a features section forwarding the store choice:

```toml
[features]
default = ["redb-store"]
redb-store = ["sapphire-ledger-core/redb-store", "sapphire-ledger-mcp/redb-store"]
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
    let out = Command::new(bin())
        .args(["mcp", "--help"])
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--init"), "mcp --help should document --init, got: {text}");
}

#[test]
fn check_reports_a_clean_empty_ledger() {
    let dir = tempfile::tempdir().expect("tempdir");
    let init = Command::new(bin())
        .args(["init"])
        .arg(dir.path())
        .output()
        .expect("run init");
    assert!(init.status.success(), "init failed: {}", String::from_utf8_lossy(&init.stderr));

    let out = Command::new(bin())
        .args(["--ledger-dir"])
        .arg(dir.path())
        .arg("check")
        .output()
        .expect("run check");
    assert!(out.status.success(), "check failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
}
```

Add to `sapphire-ledger-cli/Cargo.toml`:

```toml
[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 3: Run to confirm it fails**

```bash
cargo test -p sapphire-ledger-cli
```

Expected: `mcp_help_lists_the_init_flag` FAILS — there is no `--init` flag yet.

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

and replace its match arm:

```rust
        Command::Mcp { init } => {
            sapphire_ledger_mcp::run(cli.ledger_dir.as_deref(), init)
        }
```

`run` returns `anyhow::Result<()>`, matching `main`'s return type, so no conversion is needed.

- [ ] **Step 5: Run the tests**

```bash
cargo test -p sapphire-ledger-cli
```

Expected: 2 passed.

- [ ] **Step 6: Verify the whole workspace builds and passes**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all tests pass, no clippy warnings.

- [ ] **Step 7: Smoke-test the server by hand**

```bash
cargo run -p sapphire-ledger-cli -- mcp --help
```

Expected: help text showing `--init` and the global `--ledger-dir`.

- [ ] **Step 8: Commit**

```bash
git add sapphire-ledger-cli/Cargo.toml sapphire-ledger-cli/src/main.rs sapphire-ledger-cli/tests/cli_mcp.rs Cargo.lock
git commit -m "feat(cli): hand `sapphire-ledger mcp` to the MCP library

Replaces the not-yet-implemented bail. --init mirrors the journal CLI so an
agent can be pointed at a directory that is not a ledger yet."
```

---

### Task 9: Update the design doc and the README status

The docs describe a project that no longer matches the code. Fixing them is part of this work, not a follow-up.

**Files:**
- Modify: `docs/design.md` (the "Cache strategy", "Crate structure", "MCP server" and "Status" sections; the caretta-id wording)
- Modify: `README.md` (the "Status" and "Project structure" sections)

- [ ] **Step 1: Correct the id terminology**

In `docs/design.md`, replace every occurrence of "caretta-id" with "grain-id", and add after the first mention:

```markdown
Ids are minted by [`grain-id`](https://crates.io/crates/grain-id) via
`GrainId::now_unix()`, which has decisecond resolution. Two records minted in
the same decisecond collide; `core::ops` re-mints rather than overwriting.
```

- [ ] **Step 2: Rewrite the "Cache strategy" section**

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

- [ ] **Step 3: Correct the crate structure and MCP sections**

In "Crate structure", add the planned server crate to the tree:

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

- [ ] **Step 4: Update both Status sections**

In `docs/design.md`, replace the Status list with:

```markdown
- ✅ Workspace scaffold, `cargo build` clean.
- ✅ Data model (accounts, transactions, postings, prices, assertions, config).
- ✅ TOML round-trip with serde.
- ✅ Path conventions, workspace discovery, `init_workspace`.
- ✅ Repository I/O and cross-record validation + `sapphire-ledger check`.
- ✅ Price-log records (storage; conversion and reporting deferred).
- ✅ `core::ops` write path with grain-id.
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

- [ ] **Step 5: Verify the docs match reality**

```bash
grep -rn "caretta" docs/ README.md
grep -rn "rusqlite\|cache.sqlite" docs/design.md
```

Expected: the first returns nothing. The second returns only the lines from Step 2 that explain why rusqlite is absent.

- [ ] **Step 6: Commit**

```bash
git add docs/design.md README.md
git commit -m "docs: describe the ledger that now exists

The cache strategy, the MCP template, the crate list and both status
sections all described a plan rather than the code."
```

---

## Self-Review

**Spec coverage.** Every item in the spec's phase-1 steps 1-3 maps to a task: step 1 (`core::ops` + grain-id + the Price split) is Tasks 1, 2, 3, 4; step 2 (framework migration, dependency and state object only) is Task 5; step 3 (MCP read and write tools) is Tasks 6, 7, with the CLI entry point in Task 8. The spec's stated tool list is fully covered — `list_accounts`, `get_transaction`, `query_postings`, `validate_workspace`, `query_prices` in Task 6, and `add_transaction`, `add_account`, `add_assertion` in Task 7, which also adds `add_price` because the spec puts price *storage* in phase 1 and a write-only-by-hand record type would be inconsistent with every other kind. Step 4 (`sapphire-ledger-server`) and step 5 (agent wiring) are correctly absent. Task 9 has no matching spec step; it exists because the spec says `docs/design.md` "should be updated once this lands".

**Deliberate omissions.** `sapphire-framework-track` is declared in the workspace manifest but wired into no crate — the spec's scope note says to introduce the dependency without building indexing on it, and there is nothing to track yet. The `http-server` feature is absent throughout, per the Global Constraints.

**Type consistency.** `LedgerState::open` / `find` / `workspace` / `root` / `reload` are defined in Task 5 and used with those exact names in Tasks 6 and 7. `ops::create_transaction` returns `(String, PathBuf)` in Task 4 and is destructured as `(id, dest)` in Task 7. `Price` is defined in `prices.rs` in Task 2 and referenced as `sapphire_ledger_core::Price` in Task 7, which the Task 2 re-export provides. `EmptyParams` is defined in Task 6 and used in the Task 7 tests. `parse_date` is defined in Task 6 and used by Task 7's tools; `parse_decimal`, `parse_account_type`, `parse_status` and `build_posting` are defined in Task 7 before their first use.

**Verified assumptions.** `AppContext::new` is `const` (`crates/sapphire-framework-workspace/src/context.rs:56`, documented as "`const` so it can be used in `static` initialisers"), so the `LEDGER_CTX` static in Task 5 compiles as written. `sapphire-ledger-cli` already depends on `sapphire-ledger-mcp`, so Task 8 only adjusts that entry rather than adding it.

**Known risk.** Task 5 is the first build against the framework's git dependency, which tracks `branch = "main"` and is not pinned. If it fails to build, check what `sapphire-journal-core` currently pins before assuming the plan is wrong.
