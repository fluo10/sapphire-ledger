use std::fs;
use std::path::PathBuf;

use sapphire_ledger_core::{init_workspace, load_workspace};

fn tempdir() -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("sapphire-ledger-validate-{nanos}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_account(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn write_transaction(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn write_assertion(root: &std::path::Path, rel: &str, body: &str) {
    write_transaction(root, rel, body)
}

const ACCOUNT_CASH_JPY: &str = r#"
name = "Assets:Cash:JPY"
type = "Asset"
currencies = ["JPY"]
opened_at = "2026-05-21"
"#;

const ACCOUNT_FOOD: &str = r#"
name = "Expenses:Food"
type = "Expense"
opened_at = "2026-05-21"
"#;

#[test]
fn clean_workspace_reports_no_issues() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_account(&root, "accounts/Assets/Cash/JPY.toml", ACCOUNT_CASH_JPY);
    write_account(&root, "accounts/Expenses/Food.toml", ACCOUNT_FOOD);
    write_transaction(
        &root,
        "transactions/2026/05/tx01.toml",
        r#"
id = "tx01"
date = "2026-05-21"
narration = "lunch"
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Expenses:Food"
amount = "1000"
currency = "JPY"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-1000"
currency = "JPY"
"#,
    );

    let workspace = load_workspace(&root).unwrap();
    assert!(workspace.validate().is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn flags_undefined_account_in_transaction() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_account(&root, "accounts/Assets/Cash/JPY.toml", ACCOUNT_CASH_JPY);
    write_transaction(
        &root,
        "transactions/2026/05/bad01.toml",
        r#"
id = "bad01"
date = "2026-05-21"
narration = "typo"
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Expenses:Foood"
amount = "1000"
currency = "JPY"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-1000"
currency = "JPY"
"#,
    );

    let issues = load_workspace(&root).unwrap().validate();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("undefined account"));
    assert!(issues[0].message.contains("Expenses:Foood"));
    assert_eq!(issues[0].transaction_id.as_deref(), Some("bad01"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn flags_currency_constraint_violation() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_account(&root, "accounts/Assets/Cash/JPY.toml", ACCOUNT_CASH_JPY);
    write_account(&root, "accounts/Expenses/Food.toml", ACCOUNT_FOOD);
    write_transaction(
        &root,
        "transactions/2026/05/cur01.toml",
        r#"
id = "cur01"
date = "2026-05-21"
narration = "wrong currency"
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Expenses:Food"
amount = "10"
currency = "USD"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-10"
currency = "USD"
"#,
    );

    let issues = load_workspace(&root).unwrap().validate();
    // Cash:JPY has currencies=["JPY"], so USD posting is invalid.
    // Expenses:Food has no constraint so its USD posting is fine.
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("only allows JPY"));
    assert_eq!(issues[0].account.as_deref(), Some("Assets:Cash:JPY"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn flags_unbalanced_transaction() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_account(&root, "accounts/Assets/Cash/JPY.toml", ACCOUNT_CASH_JPY);
    write_account(&root, "accounts/Expenses/Food.toml", ACCOUNT_FOOD);
    write_transaction(
        &root,
        "transactions/2026/05/unbal.toml",
        r#"
id = "unbal"
date = "2026-05-21"
narration = "off by one"
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Expenses:Food"
amount = "1000"
currency = "JPY"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-999"
currency = "JPY"
"#,
    );

    let issues = load_workspace(&root).unwrap().validate();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("does not balance"));
    assert_eq!(issues[0].transaction_id.as_deref(), Some("unbal"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn flags_undefined_account_in_assertion() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_assertion(
        &root,
        "assertions/2026/05/as01.toml",
        r#"
id = "as01"
account = "Assets:Phantom"
date = "2026-05-31"
created_at = "2026-05-31T23:59:00+09:00"
updated_at = "2026-05-31T23:59:00+09:00"
[[balances]]
amount = "100"
currency = "JPY"
"#,
    );

    let issues = load_workspace(&root).unwrap().validate();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("undefined account"));
    assert_eq!(issues[0].assertion_id.as_deref(), Some("as01"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn reports_multiple_issues_without_short_circuiting() {
    let root = tempdir();
    init_workspace(&root, "JPY").unwrap();
    write_account(&root, "accounts/Assets/Cash/JPY.toml", ACCOUNT_CASH_JPY);
    // unbalanced
    write_transaction(
        &root,
        "transactions/2026/05/a.toml",
        r#"
id = "a"
date = "2026-05-21"
narration = ""
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Assets:Cash:JPY"
amount = "100"
currency = "JPY"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-99"
currency = "JPY"
"#,
    );
    // undefined account
    write_transaction(
        &root,
        "transactions/2026/05/b.toml",
        r#"
id = "b"
date = "2026-05-21"
narration = ""
created_at = "2026-05-21T12:00:00+09:00"
updated_at = "2026-05-21T12:00:00+09:00"
[[postings]]
account = "Mystery"
amount = "1"
currency = "JPY"
[[postings]]
account = "Assets:Cash:JPY"
amount = "-1"
currency = "JPY"
"#,
    );

    let issues = load_workspace(&root).unwrap().validate();
    assert_eq!(issues.len(), 2);
    let ids: Vec<&str> = issues.iter().filter_map(|i| i.transaction_id.as_deref()).collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));
    fs::remove_dir_all(&root).unwrap();
}
