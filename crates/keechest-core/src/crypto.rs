//! Key derivation (Argon2id) and authenticated encryption
//! (XChaCha20-Poly1305).

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id cost parameters, stored in the vault header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    /// 256 MiB, 3 passes, 4 lanes: well above OWASP's minimum and about a
    /// second on a current desktop.
    fn default() -> Self {
        Self {
            memory_kib: 256 * 1024,
            iterations: 3,
            parallelism: 4,
        }
    }
}

impl KdfParams {
    pub const MIN_MEMORY_KIB: u32 = 8 * 1024;
    pub const MAX_MEMORY_KIB: u32 = 4 * 1024 * 1024;
    pub const MAX_ITERATIONS: u32 = 100;
    pub const MAX_PARALLELISM: u32 = 64;

    /// Rejects parameters that are too weak to create a vault with, or so
    /// large that opening a crafted file would exhaust memory.
    pub fn validate(&self) -> Result<()> {
        let ok = (Self::MIN_MEMORY_KIB..=Self::MAX_MEMORY_KIB).contains(&self.memory_kib)
            && (1..=Self::MAX_ITERATIONS).contains(&self.iterations)
            && (1..=Self::MAX_PARALLELISM).contains(&self.parallelism)
            && self.memory_kib >= 8 * self.parallelism;
        if ok { Ok(()) } else { Err(Error::BadKdfParams) }
    }
}

/// Derives the vault key from the master password.
pub fn derive_key(password: &[u8], salt: &[u8; SALT_LEN], kdf: &KdfParams) -> Result<Key> {
    kdf.validate()?;
    let params = Params::new(
        kdf.memory_kib,
        kdf.iterations,
        kdf.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| Error::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|e| Error::Kdf(e.to_string()))?;
    Ok(key)
}

/// Encrypts `buf` in place and appends the tag. Reserve `TAG_LEN` spare
/// capacity first so the plaintext is not copied into a reallocation.
pub fn seal(key: &Key, nonce: &[u8; NONCE_LEN], aad: &[u8], buf: &mut Vec<u8>) -> Result<()> {
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    cipher
        .encrypt_in_place(XNonce::from_slice(nonce), aad, buf)
        .map_err(|_| Error::Encrypt)
}

/// Checks the tag and decrypts `buf` in place. Fails if the key, the AAD or
/// any ciphertext byte is wrong.
pub fn open(key: &Key, nonce: &[u8; NONCE_LEN], aad: &[u8], buf: &mut Vec<u8>) -> Result<()> {
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    cipher
        .decrypt_in_place(XNonce::from_slice(nonce), aad, buf)
        .map_err(|_| Error::WrongPasswordOrCorrupted)
}

#[cfg(test)]
pub(crate) fn test_params() -> KdfParams {
    KdfParams {
        memory_kib: KdfParams::MIN_MEMORY_KIB,
        iterations: 1,
        parallelism: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_depends_on_password_salt_and_params() {
        let p = test_params();
        let salt = [1; SALT_LEN];
        let k = derive_key(b"hunter2", &salt, &p).unwrap();
        assert_eq!(*k, *derive_key(b"hunter2", &salt, &p).unwrap());
        assert_ne!(*k, *derive_key(b"hunter3", &salt, &p).unwrap());
        assert_ne!(*k, *derive_key(b"hunter2", &[2; SALT_LEN], &p).unwrap());
        let p2 = KdfParams { iterations: 2, ..p };
        assert_ne!(*k, *derive_key(b"hunter2", &salt, &p2).unwrap());
    }

    #[test]
    fn weak_or_huge_params_are_rejected() {
        let ok = KdfParams::default();
        assert!(ok.validate().is_ok());
        for bad in [
            KdfParams {
                memory_kib: 1024,
                ..ok
            },
            KdfParams {
                memory_kib: u32::MAX,
                ..ok
            },
            KdfParams {
                iterations: 0,
                ..ok
            },
            KdfParams {
                iterations: 1000,
                ..ok
            },
            KdfParams {
                parallelism: 0,
                ..ok
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn seal_open_roundtrip_and_tamper() {
        let key = Zeroizing::new([7; KEY_LEN]);
        let nonce = [9; NONCE_LEN];
        let mut buf = b"secret".to_vec();
        seal(&key, &nonce, b"hdr", &mut buf).unwrap();
        assert_eq!(buf.len(), 6 + TAG_LEN);

        let mut copy = buf.clone();
        open(&key, &nonce, b"hdr", &mut copy).unwrap();
        assert_eq!(copy, b"secret");

        assert!(open(&key, &nonce, b"HDR", &mut buf.clone()).is_err());
        let mut flipped = buf.clone();
        flipped[0] ^= 1;
        assert!(open(&key, &nonce, b"hdr", &mut flipped).is_err());
        let other = Zeroizing::new([8; KEY_LEN]);
        assert!(open(&other, &nonce, b"hdr", &mut buf).is_err());
    }
}
