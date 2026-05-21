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

    #[error("not a sapphire-ledger workspace: no .sapphire-ledger/ found from {0}")]
    NotAWorkspace(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;
