use clap::Parser as _;
use sapphire_ledger_server::cli::{Cli, Command};
use sapphire_ledger_server::{identity, init, serve};

const LEDGER_DIR_REQUIRED: &str = "--ledger-dir is required (or set SAPPHIRE_LEDGER_SERVER_DIR)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    sapphire_ledger_core::init_app_context();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // stdout carries the raw token from `device add` and `device rotate`
        // and nothing else, so every log line goes to stderr. On Windows the
        // framework warns when it cannot restrict the key file's permissions,
        // and that warning must not land in `device add > token.txt`.
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let ledger_dir = cli
        .ledger_dir
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{LEDGER_DIR_REQUIRED}"))?;
    // Resolved per command, not once up front, because `init` must not resolve
    // it at all. `default_keys_path` goes through `cache_dir_for`, which derives
    // its directory from the *canonicalized* ledger root; `canonicalize` fails
    // on a path that does not exist yet and the framework falls back to the raw
    // path. Asking for the key file before `init` has run therefore names a
    // different directory than every command afterwards — a token written to one
    // and looked for in the other.
    let keys_path = || match cli.keys.clone() {
        Some(p) => Ok(p),
        None => serve::default_keys_path(&ledger_dir),
    };

    match cli.command {
        None => {
            let state = serve::build_state(&keys_path()?)?;
            serve::run(cli.addr, &ledger_dir, state, &cli.allowed_host).await
        }
        Some(Command::Init { base_currency }) => init::run(&ledger_dir, &base_currency),
        Some(command) => identity::run(command, &ledger_dir, &keys_path()?),
    }
}
