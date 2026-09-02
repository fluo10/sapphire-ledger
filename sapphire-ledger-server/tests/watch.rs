use sapphire_ledger_server::watch;

/// Two transactions with one id, in different months -- the shape the sync
/// model actually produces, since the path embeds the record's own date.
#[test]
fn a_duplicate_transaction_id_is_reported_with_every_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    for month in ["05", "06"] {
        let path = root.join(format!("transactions/2026/{month}/dup0001.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            r#"
id = "dup0001"
date = "2026-05-21"
narration = "same id, two months"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Food"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Cash"
amount = "-10"
currency = "JPY"
"#,
        )
        .expect("write");
    }

    let report = watch::duplicate_report(root).expect("report");
    assert_eq!(report.len(), 1, "one duplicated id, one line: {report:?}");
    let line = &report[0];
    assert!(line.contains("dup0001"), "must name the id: {line}");
    assert!(line.contains("2026/05"), "must name every path: {line}");
    assert!(line.contains("2026/06"), "must name every path: {line}");
}

#[test]
fn a_clean_ledger_reports_nothing() {
    // Two distinct transactions in the same month: an empty workspace can't
    // tell "correctly found nothing" apart from "silently broken" -- this
    // has to actually run the keying logic over more than one record and
    // still come back empty, or a bug that grouped two distinct ids
    // together would pass unnoticed.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    for id in ["tx0001", "tx0002"] {
        let path = root.join(format!("transactions/2026/05/{id}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            format!(
                r#"
id = "{id}"
date = "2026-05-21"
narration = "distinct transactions, same month"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Food"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Cash"
amount = "-10"
currency = "JPY"
"#
            ),
        )
        .expect("write");
    }

    let report = watch::duplicate_report(root).expect("report");
    assert!(
        report.is_empty(),
        "two distinct ids are not a duplicate: {report:?}"
    );
}

#[test]
fn a_duplicate_account_name_is_reported_too() {
    // Two account files at different paths naming themselves the same thing:
    // what a raced rename leaves behind.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    for leaf in ["Bar", "Foo"] {
        let path = root.join(format!("accounts/Assets/{leaf}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            format!(
                r#"
id = "acct{leaf}"
name = "Assets:Foo"
type = "Asset"
opened_at = "2026-01-01"
"#
            ),
        )
        .expect("write");
    }

    let report = watch::duplicate_report(root).expect("report");
    let line = report
        .iter()
        .find(|l| l.contains("Assets:Foo"))
        .unwrap_or_else(|| panic!("a duplicated account name must be reported: {report:?}"));
    assert!(line.contains("Bar.toml"), "must name every path: {line}");
    assert!(line.contains("Foo.toml"), "must name every path: {line}");
}

#[test]
fn a_duplicate_account_id_across_two_names_is_reported() {
    // The canonical raced-rename artifact: an account's path is derived
    // from its *name*, so a rename that races leaves one id claimed by two
    // files under two genuinely different names. Name-grouping alone would
    // report nothing here -- the two names really are distinct -- so this
    // is what only id-grouping catches.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    sapphire_ledger_core::init_workspace(root, "JPY").expect("init");

    let old_path = root.join("accounts/Assets/Cash.toml");
    let new_path = root.join("accounts/Assets/Wallet.toml");
    std::fs::create_dir_all(old_path.parent().unwrap()).expect("mkdir");
    std::fs::write(
        &old_path,
        r#"
id = "acctCash"
name = "Assets:Cash"
type = "Asset"
opened_at = "2026-01-01"
"#,
    )
    .expect("write");
    std::fs::write(
        &new_path,
        r#"
id = "acctCash"
name = "Assets:Wallet"
type = "Asset"
opened_at = "2026-01-01"
"#,
    )
    .expect("write");

    let report = watch::duplicate_report(root).expect("report");
    let line = report
        .iter()
        .find(|l| l.contains("acctCash"))
        .unwrap_or_else(|| {
            panic!("a duplicated account id across two names must be reported: {report:?}")
        });
    assert!(line.contains("Cash.toml"), "must name every path: {line}");
    assert!(line.contains("Wallet.toml"), "must name every path: {line}");
}
