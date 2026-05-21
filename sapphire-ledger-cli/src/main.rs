use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "sale",
    about = "Local-first double-entry household ledger",
    version
)]
struct Cli {
    /// Path to the ledger root (the directory containing `.sapphire-ledger/`).
    /// Overrides the automatic upward search from the current directory.
    /// Can also be set via the SAPPHIRE_LEDGER_DIR environment variable.
    #[arg(long, env = "SAPPHIRE_LEDGER_DIR", global = true, value_name = "DIR")]
    ledger_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new ledger in the given directory (defaults to current directory)
    Init {
        /// Directory to initialize (created if it does not exist)
        path: Option<PathBuf>,
        /// Base currency used for reporting (default: JPY)
        #[arg(long, default_value = "JPY")]
        base_currency: String,
    },
    /// Load every record and report any validation issues
    Check,
    /// Run the MCP server over stdio
    Mcp,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init {
            path,
            base_currency,
        } => {
            let target = path.unwrap_or_else(|| PathBuf::from("."));
            sapphire_ledger_core::init_workspace(&target, &base_currency)?;
            println!(
                "Initialized sapphire-ledger workspace at {} (base currency: {})",
                target.display(),
                base_currency
            );
            Ok(())
        }
        Command::Check => {
            let start = cli.ledger_dir.unwrap_or_else(|| PathBuf::from("."));
            let root = sapphire_ledger_core::find_workspace_root(&start)?;
            let workspace = sapphire_ledger_core::load_workspace(&root)?;
            let issues = workspace.validate();
            if issues.is_empty() {
                println!(
                    "OK: {} account(s), {} transaction(s), {} assertion(s)",
                    workspace.accounts.len(),
                    workspace.transactions.len(),
                    workspace.assertions.len()
                );
                Ok(())
            } else {
                for issue in &issues {
                    eprintln!("- {}", issue.message);
                }
                anyhow::bail!("validation failed with {} issue(s)", issues.len());
            }
        }
        Command::Mcp => {
            anyhow::bail!("mcp: not yet implemented");
        }
    }
}
