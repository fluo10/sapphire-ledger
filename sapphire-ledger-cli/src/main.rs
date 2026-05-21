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
    },
    /// Run the MCP server over stdio
    Mcp,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { path: _ } => {
            anyhow::bail!("init: not yet implemented");
        }
        Command::Mcp => {
            anyhow::bail!("mcp: not yet implemented");
        }
    }
}
