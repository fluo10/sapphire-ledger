use sapphire_ledger_core::{AccountType, LedgerState, ops};

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

    assert_eq!(
        state.workspace().accounts.len(),
        0,
        "open() takes a snapshot"
    );
    state.reload().expect("reload");
    assert_eq!(
        state.workspace().accounts.len(),
        1,
        "reload() must see the new file"
    );
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
