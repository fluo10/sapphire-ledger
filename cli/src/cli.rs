use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sapphire-ledger",
    about = "Local-first double-entry household ledger",
    version
)]
pub struct Cli {
    /// Path to the ledger root (the directory containing `.sapphire-ledger/`).
    /// Overrides the automatic upward search from the current directory.
    /// Can also be set via the SAPPHIRE_LEDGER_DIR environment variable.
    #[arg(long, env = "SAPPHIRE_LEDGER_DIR", global = true, value_name = "DIR")]
    pub ledger_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
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
    Mcp {
        /// Create the ledger if the target directory is not one yet
        #[arg(long)]
        init: bool,
    },
    /// Manage the chart of accounts.
    #[command(subcommand)]
    Account(AccountCommand),
    /// Record and read transactions.
    #[command(subcommand)]
    Tx(TxCommand),
    /// Record balance assertions.
    #[command(subcommand)]
    Assertion(AssertionCommand),
    /// Record observed exchange rates.
    #[command(subcommand)]
    Price(PriceCommand),
}

#[derive(Subcommand)]
pub enum AccountCommand {
    /// Create an account. Every account a posting names must exist first.
    Add {
        /// Colon-separated name, e.g. `Assets:Cash:JPY`.
        name: String,
        /// One of: Asset, Liability, Equity, Income, Expense.
        #[arg(long = "type", value_name = "TYPE")]
        account_type: String,
        /// Currencies this account may hold. Repeatable. Omit to allow any.
        #[arg(long = "currency", value_name = "CCY")]
        currencies: Vec<String>,
        /// `YYYY-MM-DD` the account opened. Defaults to today.
        #[arg(long, value_name = "DATE")]
        opened_at: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TxCommand {
    /// Record a transaction.
    ///
    /// Postings must sum to zero per currency, and every account named must
    /// already exist. Nothing is written if validation fails.
    Add {
        /// `YYYY-MM-DD`. Defaults to today.
        #[arg(long, value_name = "DATE")]
        date: Option<String>,
        /// What the transaction was.
        #[arg(long)]
        narration: String,
        #[arg(long)]
        payee: Option<String>,
        /// Repeatable.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// `cleared` or `pending`.
        #[arg(long)]
        status: Option<String>,
        /// `<account> <amount> <currency>`, with an optional
        /// `@ <value> <currency>` for a cross-currency inline price.
        /// Repeatable; at least two are required.
        ///
        /// The account may be given by name or by id.
        #[arg(
            long = "posting",
            value_name = "SPEC",
            required = true,
            allow_hyphen_values = true
        )]
        postings: Vec<String>,
    },
    /// List postings, newest last.
    List {
        /// Account name or id.
        #[arg(long)]
        account: Option<String>,
        #[arg(long, value_name = "CCY")]
        currency: Option<String>,
        /// Inclusive lower bound, `YYYY-MM-DD`.
        #[arg(long, value_name = "DATE")]
        date_from: Option<String>,
        /// Inclusive upper bound, `YYYY-MM-DD`.
        #[arg(long, value_name = "DATE")]
        date_to: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AssertionCommand {
    /// Record what an account should hold at the END of a date.
    Add {
        /// Account name or id.
        account: String,
        /// `YYYY-MM-DD`.
        #[arg(long, value_name = "DATE")]
        date: String,
        /// `<amount> <currency>`. Repeatable; at least one is required.
        #[arg(
            long = "balance",
            value_name = "SPEC",
            required = true,
            allow_hyphen_values = true
        )]
        balances: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum PriceCommand {
    /// Record that one unit of `base` cost `rate` units of `quote` on `date`.
    Add {
        /// `YYYY-MM-DD`.
        #[arg(long, value_name = "DATE")]
        date: String,
        #[arg(long, value_name = "CCY")]
        base: String,
        #[arg(long, value_name = "CCY")]
        quote: String,
        #[arg(long)]
        rate: String,
        /// Where the rate came from, e.g. `manual`.
        #[arg(long)]
        source: Option<String>,
    },
}
