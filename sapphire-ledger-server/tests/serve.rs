use std::net::SocketAddr;

use sapphire_ledger_server::serve;

fn addr(s: &str) -> SocketAddr {
    s.parse().expect("addr")
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
