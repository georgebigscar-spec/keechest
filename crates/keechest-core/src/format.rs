//! The vault file layout. All integers are little-endian.
//!
//! | Offset | Size | Field |
//! | --- | --- | --- |
//! | 0 | 8 | magic `KEECHEST` |
//! | 8 | 2 | format version, 1 |
//! | 10 | 1 | KDF id, 1 = Argon2id v1.3 |
//! | 11 | 4 | Argon2 memory, KiB |
//! | 15 | 4 | Argon2 iterations |
//! | 19 | 4 | Argon2 parallelism |
//! | 23 | 32 | salt |
//! | 55 | 1 | cipher id, 1 = XChaCha20-Poly1305 |
//! | 56 | 24 | nonce, fresh for every save |
//! | 80 | .. | ciphertext followed by the 16-byte tag |
//!
//! The whole header is the AEAD's associated data, so changing any of it
//! (say, to weaken the KDF parameters) makes decryption fail. The plaintext
//! is UTF-8 JSON padded with spaces to a multiple of [`PAD_TO`] bytes, so
//! the file size only reveals the vault size roughly.

use crate::crypto::{NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::{Error, KdfParams, Result};

pub const MAGIC: &[u8; 8] = b"KEECHEST";
pub const VERSION: u16 = 1;
pub const KDF_ARGON2ID: u8 = 1;
pub const CIPHER_XCHACHA20POLY1305: u8 = 1;
pub const HEADER_LEN: usize = 80;
pub const PAD_TO: usize = 1024;
/// The usual file extension, without the dot.
pub const EXTENSION: &str = "kchest";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&VERSION.to_le_bytes());
        out[10] = KDF_ARGON2ID;
        out[11..15].copy_from_slice(&self.kdf.memory_kib.to_le_bytes());
        out[15..19].copy_from_slice(&self.kdf.iterations.to_le_bytes());
        out[19..23].copy_from_slice(&self.kdf.parallelism.to_le_bytes());
        out[23..55].copy_from_slice(&self.salt);
        out[55] = CIPHER_XCHACHA20POLY1305;
        out[56..80].copy_from_slice(&self.nonce);
        out
    }

    /// Parses the header at the start of `file`, checking the magic, the
    /// version, the algorithm ids and the KDF parameter bounds.
    pub fn decode(file: &[u8]) -> Result<Self> {
        if file.len() < 8 || &file[0..8] != MAGIC {
            return Err(Error::NotAVault);
        }
        if file.len() < HEADER_LEN + TAG_LEN {
            return Err(Error::Corrupted("file is truncated".into()));
        }
        let u32_at = |i: usize| u32::from_le_bytes(file[i..i + 4].try_into().unwrap());
        let version = u16::from_le_bytes([file[8], file[9]]);
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        if file[10] != KDF_ARGON2ID || file[55] != CIPHER_XCHACHA20POLY1305 {
            return Err(Error::UnsupportedAlgorithm);
        }
        let kdf = KdfParams {
            memory_kib: u32_at(11),
            iterations: u32_at(15),
            parallelism: u32_at(19),
        };
        kdf.validate()?;
        Ok(Self {
            kdf,
            salt: file[23..55].try_into().unwrap(),
            nonce: file[56..80].try_into().unwrap(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            kdf: KdfParams::default(),
            salt: [3; SALT_LEN],
            nonce: [4; NONCE_LEN],
        }
    }

    fn file() -> Vec<u8> {
        let mut f = header().encode().to_vec();
        f.extend_from_slice(&[0; TAG_LEN]);
        f
    }

    #[test]
    fn roundtrip() {
        assert_eq!(Header::decode(&file()).unwrap(), header());
    }

    #[test]
    fn rejects_bad_headers() {
        assert!(matches!(Header::decode(b"hello"), Err(Error::NotAVault)));
        assert!(matches!(
            Header::decode(&file()[..HEADER_LEN]),
            Err(Error::Corrupted(_))
        ));
        let mut f = file();
        f[8] = 9;
        assert!(matches!(
            Header::decode(&f),
            Err(Error::UnsupportedVersion(9))
        ));
        let mut f = file();
        f[55] = 2;
        assert!(matches!(
            Header::decode(&f),
            Err(Error::UnsupportedAlgorithm)
        ));
        let mut f = file();
        f[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Header::decode(&f), Err(Error::BadKdfParams)));
    }
}
