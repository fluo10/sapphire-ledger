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
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");
    assert!(
        watch::duplicate_report(dir.path())
            .expect("report")
            .is_empty()
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
    assert!(
        report.iter().any(|l| l.contains("Assets:Foo")),
        "a duplicated account name must be reported: {report:?}"
    );
}
