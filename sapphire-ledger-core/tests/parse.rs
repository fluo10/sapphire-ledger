use rust_decimal::Decimal;
use std::str::FromStr;

use sapphire_ledger_core::{
    Account, AccountType, Assertion, Balance, CacheConfig, Config, Transaction, TransactionStatus,
};

fn parse_transaction(toml_str: &str) -> Transaction {
    toml::from_str(toml_str).expect("transaction parse")
}

#[test]
fn transaction_roundtrip_single_currency() {
    let input = r#"
id = "0a1b2c3"
date = "2026-05-21"
narration = "イオン買い物"
payee = "イオン"
tags = ["grocery"]
status = "cleared"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account = "Expenses:Food"
amount = "1200"
currency = "JPY"

[[postings]]
account = "Expenses:Daily"
amount = "800"
currency = "JPY"
memo = "洗剤"

[[postings]]
account = "Liabilities:CreditCard:Rakuten"
amount = "-2000"
currency = "JPY"
"#;
    let tx = parse_transaction(input);
    assert_eq!(tx.id, "0a1b2c3");
    assert_eq!(tx.narration, "イオン買い物");
    assert_eq!(tx.payee.as_deref(), Some("イオン"));
    assert_eq!(tx.status, Some(TransactionStatus::Cleared));
    assert_eq!(tx.postings.len(), 3);
    assert_eq!(tx.postings[1].memo.as_deref(), Some("洗剤"));
    tx.validate().expect("balanced");

    // Round-trip
    let serialized = toml::to_string(&tx).expect("serialize");
    let reparsed: Transaction = toml::from_str(&serialized).expect("reparse");
    assert_eq!(tx, reparsed);
}

#[test]
fn transaction_balance_multi_currency_with_price() {
    let input = r#"
id = "fx0001"
date = "2026-05-21"
narration = "JPY to USD"
created_at = "2026-05-21T10:00:00+09:00"
updated_at = "2026-05-21T10:00:00+09:00"

[[postings]]
account = "Assets:Cash:USD"
amount = "100"
currency = "USD"
price = { value = "150", currency = "JPY" }

[[postings]]
account = "Assets:Cash:JPY"
amount = "-15000"
currency = "JPY"
"#;
    let tx = parse_transaction(input);
    tx.validate().expect("balanced after price conversion");
}

#[test]
fn transaction_rejects_unbalanced() {
    let input = r#"
id = "bad01"
date = "2026-05-21"
narration = "broken"
created_at = "2026-05-21T10:00:00+09:00"
updated_at = "2026-05-21T10:00:00+09:00"

[[postings]]
account = "Expenses:Food"
amount = "1200"
currency = "JPY"

[[postings]]
account = "Assets:Cash:JPY"
amount = "-1000"
currency = "JPY"
"#;
    let err = parse_transaction(input).validate().unwrap_err();
    assert!(format!("{err}").contains("does not balance"));
}

#[test]
fn transaction_rejects_single_posting() {
    let input = r#"
id = "lone"
date = "2026-05-21"
narration = "only one side"
created_at = "2026-05-21T10:00:00+09:00"
updated_at = "2026-05-21T10:00:00+09:00"

[[postings]]
account = "Expenses:Food"
amount = "0"
currency = "JPY"
"#;
    let err = parse_transaction(input).validate().unwrap_err();
    assert!(format!("{err}").contains("fewer than 2"));
}

#[test]
fn account_roundtrip() {
    let input = r#"
name = "Assets:Cash:USD"
type = "Asset"
currencies = ["USD"]
opened_at = "2026-05-21"
description = "ドル現金"
"#;
    let account: Account = toml::from_str(input).expect("account parse");
    assert_eq!(account.account_type, AccountType::Asset);
    assert!(account.allows_currency("USD"));
    assert!(!account.allows_currency("JPY"));

    let serialized = toml::to_string(&account).expect("serialize");
    let reparsed: Account = toml::from_str(&serialized).expect("reparse");
    assert_eq!(account, reparsed);
}

#[test]
fn account_allows_any_currency_when_unspecified() {
    let input = r#"
name = "Equity:OpeningBalances"
type = "Equity"
opened_at = "2026-05-21"
"#;
    let account: Account = toml::from_str(input).expect("parse");
    assert!(account.currencies.is_empty());
    assert!(account.allows_currency("JPY"));
    assert!(account.allows_currency("USD"));
}

#[test]
fn assertion_roundtrip_multi_currency() {
    let input = r#"
id = "as0001"
account = "Assets:Brokerage"
date = "2026-05-31"
created_at = "2026-05-31T23:59:00+09:00"
updated_at = "2026-05-31T23:59:00+09:00"

[[balances]]
amount = "100"
currency = "USD"

[[balances]]
amount = "5000"
currency = "JPY"
"#;
    let assertion: Assertion = toml::from_str(input).expect("assertion parse");
    assert_eq!(assertion.balances.len(), 2);
    assert_eq!(
        assertion.balances[0],
        Balance {
            amount: Decimal::from_str("100").unwrap(),
            currency: "USD".into()
        }
    );

    let serialized = toml::to_string(&assertion).expect("serialize");
    let reparsed: Assertion = toml::from_str(&serialized).expect("reparse");
    assert_eq!(assertion, reparsed);
}

#[test]
fn config_defaults_cache_section() {
    let input = r#"
schema_version = 1
base_currency = "JPY"
"#;
    let config: Config = toml::from_str(input).expect("config parse");
    assert_eq!(config.base_currency, "JPY");
    assert_eq!(config.cache, CacheConfig::default());
    assert_eq!(config.cache.scan_interval, 60);
}

#[test]
fn workspace_root_discovery() {
    use std::fs;
    let tmp = tempdir_in_target();
    let nested = tmp.join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(tmp.join(".sapphire-ledger")).unwrap();

    let found = sapphire_ledger_core::find_workspace_root(&nested).expect("found");
    assert_eq!(found, tmp.canonicalize().unwrap());

    fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn workspace_root_not_found() {
    use std::fs;
    let tmp = tempdir_in_target();
    fs::create_dir_all(&tmp).unwrap();
    let err = sapphire_ledger_core::find_workspace_root(&tmp).unwrap_err();
    assert!(format!("{err}").contains("not a sapphire-ledger workspace"));
    fs::remove_dir_all(&tmp).unwrap();
}

fn tempdir_in_target() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("sapphire-ledger-test-{nanos}"));
    path
}
