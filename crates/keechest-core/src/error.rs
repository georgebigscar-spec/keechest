use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("not a KeeChest vault")]
    NotAVault,
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u16),
    #[error("unsupported key derivation or cipher")]
    UnsupportedAlgorithm,
    #[error("key derivation parameters out of range")]
    BadKdfParams,
    #[error("key derivation failed: {0}")]
    Kdf(String),
    #[error("wrong master password or damaged file")]
    WrongPasswordOrCorrupted,
    #[error("vault contents are damaged: {0}")]
    Corrupted(String),
    #[error("encryption failed")]
    Encrypt,
    #[error("can't import: {0}")]
    Import(String),
}
