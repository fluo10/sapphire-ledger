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

/// `load_workspace` is fail-fast across four sweeps, so one malformed file
/// aborts the whole load. The MCP write tools reload before writing and the
/// `validate_workspace` tool reloads to report -- so this error string is what
/// an agent has to act on, and it is useless without the filename.
#[test]
fn a_malformed_record_names_its_file_in_the_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    sapphire_ledger_core::init_workspace(dir.path(), "JPY").expect("init");

    let broken = dir.path().join("transactions/2026/05/broken.toml");
    std::fs::create_dir_all(broken.parent().unwrap()).expect("mkdir");
    std::fs::write(&broken, "id = \"broken\"\nthis is not toml\n").expect("write");

    let err = sapphire_ledger_core::load_workspace(dir.path())
        .expect_err("a malformed record must fail the load");
    assert!(
        err.to_string().contains("broken.toml"),
        "the error must name the offending file, got: {err}"
    );
    let chain = chain(&err);
    assert!(
        chain.contains("TOML parse error"),
        "the chain must keep saying what went wrong, got: {chain}"
    );
}

/// Render an error the way a chain-aware caller does -- `{:#}` on an
/// `anyhow::Error`, or `Debug` on one. `sapphire-ledger-core` has no anyhow
/// dependency, so the walk is spelled out here.
fn chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut cursor = err.source();
    while let Some(next) = cursor {
        out.push_str(&format!(": {next}"));
        cursor = next.source();
    }
    out
}

#[test]
fn a_missing_config_names_the_file_in_the_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".sapphire-ledger")).expect("mkdir");

    let err = sapphire_ledger_core::load_workspace(dir.path())
        .expect_err("a workspace with no config must fail the load");
    let msg = err.to_string();
    assert!(
        msg.contains("config.toml"),
        "the error must name the missing file, got: {msg}"
    );
}
