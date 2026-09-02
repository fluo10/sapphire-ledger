use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_sapphire-ledger")
}

#[test]
fn mcp_help_lists_the_init_flag() {
    let out = Command::new(bin())
        .args(["mcp", "--help"])
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("--init"),
        "mcp --help should document --init, got: {text}"
    );
}

#[test]
fn check_reports_a_clean_empty_ledger() {
    let dir = tempfile::tempdir().expect("tempdir");
    let init = Command::new(bin())
        .arg("init")
        .arg(dir.path())
        .output()
        .expect("run init");
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let out = Command::new(bin())
        .arg("--ledger-dir")
        .arg(dir.path())
        .arg("check")
        .output()
        .expect("run check");
    assert!(
        out.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK"));
}
