//! The non-UI part of KeeChest: the encrypted vault file, the data model, and
//! password generation and strength estimation.
//!
//! A vault is a JSON document encrypted with XChaCha20-Poly1305 under a key
//! derived from the master password with Argon2id. See [`format`] for the
//! byte layout. Secrets are wiped from memory on drop with [`zeroize`].

pub mod crypto;
pub mod error;
pub mod format;
pub mod generator;
pub mod interop;
pub mod random;
pub mod store;
pub mod strength;
pub mod vault;

pub use crypto::KdfParams;
pub use error::{Error, Result};
pub use generator::GeneratorOptions;
pub use store::VaultFile;
pub use strength::Strength;
pub use vault::{CustomField, Entry, HistoryItem, Vault};
pub use zeroize::Zeroizing;
