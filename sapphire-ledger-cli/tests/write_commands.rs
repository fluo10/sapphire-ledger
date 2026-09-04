//! The write commands, driven through the real binary.
//!
//! At this level because that is what a person actually runs, and because the
//! point of these commands is that they are thin: the rules live in
//! `core::ops`, and what is worth testing here is that the CLI hands `ops` the
//! right things and reports what came back.

use std::path::Path;
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sapphire-ledger")
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .arg("--ledger-dir")
        .arg(root)
        .args(args)
        .output()
        .expect("run")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let out = run(root, args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ledger() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(bin())
        .arg("init")
        .arg(dir.path())
        .output()
        .expect("init");
    assert!(out.status.success(), "init failed");
    dir
}

/// Two accounts and one balanced transaction between them.
fn seeded() -> tempfile::TempDir {
    let dir = ledger();
    let root = dir.path();
    ok(
        root,
        &["account", "add", "Expenses:Food", "--type", "Expense"],
    );
    ok(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);
    ok(
        root,
        &[
            "tx",
            "add",
            "--date",
            "2026-05-21",
            "--narration",
            "groceries",
            "--posting",
            "Expenses:Food 1200 JPY",
            "--posting",
            "Assets:Cash -1200 JPY",
        ],
    );
    dir
}

#[test]
fn a_ledger_written_entirely_from_the_cli_passes_check() {
    let dir = seeded();
    let out = ok(dir.path(), &["check"]);
    assert!(out.contains("OK"), "check should be clean, got: {out}");
    assert!(
        out.contains("2 account"),
        "check should see both accounts, got: {out}"
    );
}

#[test]
fn tx_add_reports_the_id_and_the_path() {
    let dir = ledger();
    let root = dir.path();
    ok(
        root,
        &["account", "add", "Expenses:Food", "--type", "Expense"],
    );
    ok(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);

    let out = ok(
        root,
        &[
            "tx",
            "add",
            "--date",
            "2026-05-21",
            "--narration",
            "groceries",
            "--posting",
            "Expenses:Food 1200 JPY",
            "--posting",
            "Assets:Cash -1200 JPY",
        ],
    );

    assert!(out.contains("created transaction"), "got: {out}");
    assert!(
        out.contains("transactions"),
        "the path is how you find the file you just wrote, got: {out}"
    );
}

#[test]
fn an_unbalanced_transaction_is_refused_and_writes_nothing() {
    let dir = ledger();
    let root = dir.path();
    ok(
        root,
        &["account", "add", "Expenses:Food", "--type", "Expense"],
    );
    ok(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);

    let out = run(
        root,
        &[
            "tx",
            "add",
            "--date",
            "2026-05-21",
            "--narration",
            "wrong",
            "--posting",
            "Expenses:Food 1200 JPY",
            "--posting",
            "Assets:Cash -999 JPY",
        ],
    );

    assert!(!out.status.success(), "an unbalanced transaction must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does not balance"), "got: {err}");

    let month = root.join("transactions/2026/05");
    assert_eq!(
        std::fs::read_dir(&month).map(|d| d.count()).unwrap_or(0),
        0,
        "a rejected transaction must leave no file behind"
    );
}

#[test]
fn a_transaction_naming_an_unknown_account_is_refused() {
    let dir = ledger();
    let root = dir.path();
    ok(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);

    let out = run(
        root,
        &[
            "tx",
            "add",
            "--narration",
            "orphan",
            "--posting",
            "Expenses:Nowhere 10 JPY",
            "--posting",
            "Assets:Cash -10 JPY",
        ],
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("undefined account"),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn account_add_refuses_a_duplicate_name() {
    let dir = ledger();
    let root = dir.path();
    ok(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);

    let out = run(root, &["account", "add", "Assets:Cash", "--type", "Asset"]);
    assert!(!out.status.success(), "a duplicate name must be refused");
}

#[test]
fn account_add_rejects_an_unknown_type() {
    let dir = ledger();
    let out = run(
        dir.path(),
        &["account", "add", "Assets:Cash", "--type", "Bogus"],
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Bogus"),
        "the error should name the bad input"
    );
}

#[test]
fn assertion_and_price_add_land_and_check_stays_clean() {
    let dir = seeded();
    let root = dir.path();

    ok(
        root,
        &[
            "assertion",
            "add",
            "Assets:Cash",
            "--date",
            "2026-05-31",
            "--balance",
            "-1200 JPY",
        ],
    );
    ok(
        root,
        &[
            "price",
            "add",
            "--date",
            "2026-05-21",
            "--base",
            "USD",
            "--quote",
            "JPY",
            "--rate",
            "150",
        ],
    );

    let out = ok(root, &["check"]);
    assert!(out.contains("OK"), "got: {out}");
    assert!(out.contains("1 assertion"), "got: {out}");
}

#[test]
fn tx_list_shows_what_was_written_and_filters_by_account() {
    let dir = seeded();
    let root = dir.path();

    let all = ok(root, &["tx", "list"]);
    assert!(all.contains("groceries"), "got: {all}");

    let filtered = ok(root, &["tx", "list", "--account", "Expenses:Food"]);
    assert!(filtered.contains("groceries"), "got: {filtered}");

    let other = ok(root, &["tx", "list", "--account", "Assets:Cash"]);
    assert!(other.contains("groceries"), "got: {other}");

    let none = ok(root, &["tx", "list", "--date-from", "2027-01-01"]);
    assert!(
        !none.contains("groceries"),
        "a date filter past the record must exclude it, got: {none}"
    );
}

#[test]
fn a_cross_currency_posting_balances_through_its_inline_price() {
    let dir = ledger();
    let root = dir.path();
    ok(
        root,
        &["account", "add", "Assets:Cash:USD", "--type", "Asset"],
    );
    ok(
        root,
        &["account", "add", "Assets:Cash:JPY", "--type", "Asset"],
    );

    ok(
        root,
        &[
            "tx",
            "add",
            "--date",
            "2026-05-21",
            "--narration",
            "bought dollars",
            "--posting",
            "Assets:Cash:USD 100 USD @ 150 JPY",
            "--posting",
            "Assets:Cash:JPY -15000 JPY",
        ],
    );

    let out = ok(root, &["check"]);
    assert!(out.contains("OK"), "got: {out}");
}
