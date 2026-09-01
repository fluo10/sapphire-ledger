use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("validation error: {0}")]
    Validation(String),

    /// A record file that could not be read or parsed, named.
    ///
    /// `load_workspace` is fail-fast across every record in the ledger, and
    /// the MCP tools hand this text straight to an agent that can only act on
    /// what the message says -- so the path is the actionable half, and
    /// without it a parse error names a line number in a file nobody can
    /// identify.
    ///
    /// This is a context wrapper in the `anyhow` sense: it contributes the
    /// path, and the reason stays in `source`. **Render the chain**, not just
    /// this error -- `format!("{:#}", anyhow::Error::from(e))`, or `Debug` on
    /// an `anyhow::Error`, or a hand-rolled walk over `source()`. Printing
    /// this variant alone gives you a bare path. Folding the reason into the
    /// message instead was tried and rejected: every chain-aware renderer then
    /// prints the whole parse error three times over.
    #[error("{}", path.display())]
    Record {
        path: PathBuf,
        #[source]
        source: Box<Error>,
    },

    #[error("not a sapphire-ledger workspace: no .sapphire-ledger/ found from {0}")]
    NotAWorkspace(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;
