use std::net::SocketAddr;
use std::path::Path;
use std::sync::OnceLock;

use sapphire_ledger_server::serve;

fn addr(s: &str) -> SocketAddr {
    s.parse().expect("addr")
}

/// Point `LEDGER_CTX`'s cache at a throwaway directory, once per test
/// binary, and hand back that directory.
///
/// `set_cache_dir` is first-writer-wins on a process-global, and `cargo
/// test` runs the tests in this file as threads of one process, so the
/// `OnceLock` is what makes every caller agree on the same answer instead of
/// racing. It also keeps the tests out of the real user cache: without it,
/// `default_keys_path` would create directories under
/// `~/.cache/sapphire-ledger` on whoever's machine ran the suite.
fn test_cache_root() -> &'static Path {
    static CACHE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = CACHE.get_or_init(|| tempfile::tempdir().expect("cache tempdir"));
    sapphire_ledger_core::LEDGER_CTX.set_cache_dir(dir.path().to_path_buf());
    dir.path()
}

#[test]
fn loopback_needs_no_allowed_host() {
    serve::check_exposure(addr("127.0.0.1:3838"), &[])
        .expect("a loopback bind is complete on its own");
}

#[test]
fn a_wide_bind_without_an_allowed_host_is_refused() {
    let err = serve::check_exposure(addr("0.0.0.0:3838"), &[])
        .expect_err("a wide bind with no allowlist serves 403 to everyone");
    let msg = err.to_string();
    assert!(
        msg.contains("--allowed-host"),
        "the error must name the flag that fixes it, got: {msg}"
    );
}

#[test]
fn a_wide_bind_with_an_allowed_host_is_accepted() {
    serve::check_exposure(addr("0.0.0.0:3838"), &["ledger.example.net".to_string()])
        .expect("a named host makes a wide bind usable");
}

#[test]
fn a_blank_allowed_host_does_not_count() {
    serve::check_exposure(addr("0.0.0.0:3838"), &["  ".to_string()])
        .expect_err("whitespace is not a hostname");
}

#[test]
fn state_without_a_key_file_still_builds_but_holds_no_keys() {
    // The framework's `protect()` refuses every request when no key store is
    // configured, rather than passing them through. Building the state must
    // not be what fails — the refusal has to happen at request time, where an
    // operator can see it.
    let dir = tempfile::tempdir().expect("tempdir");
    let state = serve::build_state(&dir.path().join("keys.toml")).expect("state builds");
    assert!(
        state.keys().is_some(),
        "an empty key store is still a key store"
    );
}

#[test]
fn two_ledgers_on_one_host_do_not_share_a_key_file() {
    // `protect()` only checks that a bearer names *a* key in the store; it
    // never cross-checks that key's `device_id` against the workspace being
    // served. So a shared key file is a shared credential: a device
    // registered under a personal ledger would authenticate in full against
    // a business one on the same machine, and a `device_id` minted in the
    // first would land in the second's records naming nothing local --
    // exactly the unattributable state this branch's design exists to
    // prevent.
    let cache = test_cache_root();
    let personal = tempfile::tempdir().expect("tempdir");
    let business = tempfile::tempdir().expect("tempdir");

    let a = serve::default_keys_path(personal.path()).expect("keys path");
    let b = serve::default_keys_path(business.path()).expect("keys path");

    assert_ne!(
        a, b,
        "two ledgers on one host must not be handed the same key file"
    );
    assert_ne!(
        a,
        cache.join("keys.toml"),
        "the key file belongs in this ledger's own cache subdirectory, not the \
         app-wide cache root"
    );
    assert_eq!(a.file_name().expect("file name"), "keys.toml");
    assert!(
        a.parent().expect("parent").is_dir(),
        "the containing directory must exist -- `KeyStore` writes into it"
    );

    // Same ledger, same file: the path is derived from the root, not
    // generated, or `--keys`-less invocations would each mint their own.
    let again = serve::default_keys_path(personal.path()).expect("keys path");
    assert_eq!(
        a, again,
        "the same ledger must resolve to the same key file"
    );
}
