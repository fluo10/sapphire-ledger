use std::path::Path;
use std::sync::OnceLock;

use sapphire_ledger_server::{init, serve};

/// Point `LEDGER_CTX`'s cache at a throwaway directory, once per test binary.
///
/// `set_cache_dir` is first-writer-wins on a process-global and `cargo test`
/// runs this file's tests as threads of one process, so the `OnceLock` is what
/// makes every caller agree instead of racing. It also keeps the suite out of
/// the real user cache.
fn test_cache_root() -> &'static Path {
    static CACHE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = CACHE.get_or_init(|| tempfile::tempdir().expect("cache tempdir"));
    sapphire_ledger_core::LEDGER_CTX.set_cache_dir(dir.path().to_path_buf());
    dir.path()
}

#[test]
fn init_creates_the_marker_and_every_record_directory() {
    let parent = tempfile::tempdir().expect("tempdir");
    let root = parent.path().join("my-ledger");

    init::run(&root, "JPY").expect("init");

    assert!(
        root.join(".sapphire-ledger").is_dir(),
        "the marker directory is what makes this a workspace at all"
    );
    for dir in ["accounts", "transactions", "assertions", "prices"] {
        assert!(root.join(dir).is_dir(), "{dir}/ is missing");
    }
}

#[test]
fn init_writes_the_requested_base_currency() {
    let parent = tempfile::tempdir().expect("tempdir");
    let root = parent.path().join("my-ledger");

    init::run(&root, "USD").expect("init");

    let config =
        std::fs::read_to_string(root.join(".sapphire-ledger/config.toml")).expect("config");
    assert!(
        config.contains("USD"),
        "--base-currency must reach the config, got: {config}"
    );
}

#[test]
fn init_refuses_an_existing_workspace() {
    let parent = tempfile::tempdir().expect("tempdir");
    let root = parent.path().join("my-ledger");
    init::run(&root, "JPY").expect("first init");

    let err = init::run(&root, "JPY").expect_err("a second init must be refused");
    // Rendered as a chain, not `to_string()`: `run` wraps the core error in
    // context naming the path, which earns its place for the I/O failures core
    // reports without one — but that wrapper is what `to_string()` shows, so the
    // reason itself lives one link down. `main` returns `anyhow::Result`, which
    // prints the whole chain, so this is what an operator actually reads.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("already contains"),
        "the error should say the directory is already a workspace, got: {chain}"
    );
}

/// The reason `main` must not resolve the key path before dispatching `init`.
///
/// `default_keys_path` goes through `cache_dir_for`, which derives a uuid from
/// the *canonicalized* root. `canonicalize` fails on a path that does not exist
/// yet and the framework falls back to the raw path, so asking for the key file
/// before the workspace exists yields a different directory than every command
/// afterwards will use.
///
/// This pins the hazard rather than the fix: `main`'s ordering is not reachable
/// from a test. If someone moves the resolution back above the match, this test
/// still passes — but it is here so the next reader can see why the ordering is
/// deliberate instead of rediscovering it with a key file nobody can find.
#[test]
fn the_key_path_only_settles_once_the_workspace_exists() {
    test_cache_root();
    let parent = tempfile::tempdir().expect("tempdir");
    let root = parent.path().join("my-ledger");

    let before = serve::default_keys_path(&root).expect("before");
    init::run(&root, "JPY").expect("init");
    let after = serve::default_keys_path(&root).expect("after");

    assert_ne!(
        before, after,
        "if these ever agree the hazard is gone and this test can go with it"
    );

    let again = serve::default_keys_path(&root).expect("again");
    assert_eq!(
        after, again,
        "once the workspace exists the key path must be stable, or `device add` \
         and `serve` would disagree about where the token lives"
    );
}
