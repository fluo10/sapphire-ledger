use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sapphire-ledger-server",
    about = "Self-hosted MCP server for sapphire-ledger",
    version
)]
pub struct Cli {
    /// Path to the ledger root (the directory containing `.sapphire-ledger/`).
    #[arg(
        long,
        env = "SAPPHIRE_LEDGER_SERVER_DIR",
        global = true,
        value_name = "DIR"
    )]
    pub ledger_dir: Option<PathBuf>,

    /// Key file. Defaults to `keys.toml` in this app's cache directory.
    #[arg(long, global = true, value_name = "FILE")]
    pub keys: Option<PathBuf>,

    /// Address to bind. Loopback by default; widening it requires
    /// `--allowed-host`.
    #[arg(long, default_value = "127.0.0.1:3838")]
    pub addr: SocketAddr,

    /// A hostname clients use to reach this server. Repeatable. Loopback is
    /// always allowed and never needs listing.
    #[arg(long, value_name = "HOST")]
    pub allowed_host: Vec<String>,

    /// Omit to serve.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage the people devices belong to.
    #[command(subcommand)]
    User(UserCommand),
    /// Manage the clients that may reach this server.
    #[command(subcommand)]
    Device(DeviceCommand),
}

#[derive(Subcommand)]
pub enum UserCommand {
    /// Register a person or an agent.
    Add {
        name: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// List users.
    List,
}

#[derive(Subcommand)]
pub enum DeviceCommand {
    /// Register a device and mint its token. The token is printed once, to
    /// stdout, and is not recoverable afterwards.
    Add {
        name: String,
        /// The user this device belongs to, by name or id.
        #[arg(long)]
        user: String,
        #[arg(long)]
        description: Option<String>,
        /// Expire the token after this long, e.g. `90d`, `12h`.
        #[arg(long, value_name = "DURATION")]
        expires_in: Option<String>,
    },
    /// List devices, with their user and their token masked.
    List,
    /// Re-issue a device's token. The device keeps its id.
    ///
    /// This REPLACES the expiry rather than carrying the old one over:
    /// omitting the flag makes the new token non-expiring.
    Rotate {
        /// The device, by name or id.
        selector: String,
        #[arg(long, value_name = "DURATION")]
        expires_in: Option<String>,
    },
    /// Retire a device: tombstone it and revoke its key. The id still
    /// resolves to a name afterwards, so records that named it stay readable.
    Retire {
        /// The device, by name or id.
        selector: String,
    },
}
