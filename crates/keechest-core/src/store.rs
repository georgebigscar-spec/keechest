//! Reading and writing vault files.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::crypto::{self, Key, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::format::{HEADER_LEN, Header, PAD_TO};
use crate::{Error, KdfParams, Result, Vault, random};

/// An open vault file: where it lives and the derived key, kept so that
/// saving does not rerun the KDF. The key is wiped on drop.
pub struct VaultFile {
    path: PathBuf,
    key: Key,
    salt: [u8; SALT_LEN],
    kdf: KdfParams,
}

impl VaultFile {
    /// Creates a new vault file at `path` (replacing any file there) and
    /// writes `vault` to it.
    pub fn create(
        path: impl Into<PathBuf>,
        password: &str,
        kdf: KdfParams,
        vault: &Vault,
    ) -> Result<Self> {
        let file = Self::with_password(path, password, kdf)?;
        file.save(vault)?;
        Ok(file)
    }

    /// Derives a key for `password` with a fresh salt, writing nothing. This
    /// is the slow part of [`create`](Self::create), split out so it can run
    /// off the UI thread; [`save`](Self::save) the result after.
    pub fn with_password(path: impl Into<PathBuf>, password: &str, kdf: KdfParams) -> Result<Self> {
        let salt = random::array();
        let key = crypto::derive_key(password.as_bytes(), &salt, &kdf)?;
        Ok(Self {
            path: path.into(),
            key,
            salt,
            kdf,
        })
    }

    /// Opens and decrypts the vault at `path`.
    pub fn open(path: impl Into<PathBuf>, password: &str) -> Result<(Self, Vault)> {
        let path = path.into();
        let bytes = fs::read(&path)?;
        let header = Header::decode(&bytes)?;
        let key = crypto::derive_key(password.as_bytes(), &header.salt, &header.kdf)?;
        let vault = decrypt(&key, &header, &bytes)?;
        let file = Self {
            path,
            key,
            salt: header.salt,
            kdf: header.kdf,
        };
        Ok((file, vault))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kdf(&self) -> KdfParams {
        self.kdf
    }

    /// Encrypts `vault` with a fresh nonce and replaces the file atomically.
    /// The previous version is kept next to it as `<name>.bak`.
    pub fn save(&self, vault: &Vault) -> Result<()> {
        let bytes = encrypt(&self.key, &self.salt, &self.kdf, vault)?;
        write_atomic(&self.path, &bytes)
    }

    /// Re-keys the vault with a new password and KDF parameters and saves it,
    /// backup included. On failure the old key stays in use.
    pub fn change_password(&mut self, password: &str, kdf: KdfParams, vault: &Vault) -> Result<()> {
        let new = Self::with_password(self.path.clone(), password, kdf)?;
        self.replace_key(new, vault)
    }

    /// Switches to the key of `new` (from [`with_password`](Self::with_password)
    /// on the same path) and saves. On failure the old key stays in use.
    pub fn replace_key(&mut self, new: VaultFile, vault: &Vault) -> Result<()> {
        new.save(vault)?;
        *self = new;
        // The backup still opens with the old password, which may be why it
        // is being changed. Replace it with the re-keyed file.
        fs::copy(&self.path, with_suffix(&self.path, ".bak"))?;
        Ok(())
    }

    /// Checks `password` against this vault without touching the file.
    pub fn verify_password(&self, password: &str) -> Result<bool> {
        let key = crypto::derive_key(password.as_bytes(), &self.salt, &self.kdf)?;
        Ok(constant_time_eq(key.as_ref(), self.key.as_ref()))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Counts bytes, to size the plaintext buffer before writing secrets to it.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The whole file for `vault`: header, then the padded, encrypted JSON.
pub fn encrypt(
    key: &Key,
    salt: &[u8; SALT_LEN],
    kdf: &KdfParams,
    vault: &Vault,
) -> Result<Vec<u8>> {
    let header = Header {
        kdf: *kdf,
        salt: *salt,
        nonce: random::array::<NONCE_LEN>(),
    }
    .encode();

    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, vault).map_err(|e| Error::Corrupted(e.to_string()))?;
    let padded = counter.0.div_ceil(PAD_TO).max(1) * PAD_TO;
    // Allocate once, with room for the tag, so the plaintext never moves to
    // a reallocated buffer that would be freed without being wiped.
    let mut buf = Zeroizing::new(Vec::with_capacity(padded + TAG_LEN));
    serde_json::to_writer(&mut *buf, vault).map_err(|e| Error::Corrupted(e.to_string()))?;
    // JSON allows trailing whitespace.
    buf.resize(padded, b' ');
    crypto::seal(key, &header_nonce(&header), &header, &mut buf)?;

    let mut out = Vec::with_capacity(HEADER_LEN + buf.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&buf);
    Ok(out)
}

fn header_nonce(header: &[u8; HEADER_LEN]) -> [u8; NONCE_LEN] {
    header[56..80].try_into().unwrap()
}

/// Decrypts a whole vault file whose header has been parsed.
pub fn decrypt(key: &Key, header: &Header, file: &[u8]) -> Result<Vault> {
    let mut buf = Zeroizing::new(file[HEADER_LEN..].to_vec());
    crypto::open(key, &header.nonce, &file[..HEADER_LEN], &mut buf)?;
    serde_json::from_slice(&buf).map_err(|e| Error::Corrupted(e.to_string()))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// Writes to a temporary file beside `path`, syncs it, backs up the current
/// file and renames the new one over it, so a crash never leaves a
/// half-written vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if path.exists() {
            fs::copy(path, with_suffix(path, ".bak"))?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    #[cfg(unix)]
    {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Entry;
    use crate::crypto::test_params;

    fn sample() -> Vault {
        let mut v = Vault::new("Личное");
        let mut e = Entry::new();
        e.title = "Почта".into();
        e.username = "george".into();
        e.password = "correct horse battery staple".into();
        e.notes = "line 1\nline \"2\"".into();
        v.upsert(e);
        v
    }

    #[test]
    fn create_open_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kchest");
        let vault = sample();
        let file = VaultFile::create(&path, "master", test_params(), &vault).unwrap();

        let (opened, got) = VaultFile::open(&path, "master").unwrap();
        assert!(got == vault);
        assert_eq!(opened.kdf(), test_params());

        let mut changed = got.clone();
        changed.name = "Другое".into();
        file.save(&changed).unwrap();
        assert!(VaultFile::open(&path, "master").unwrap().1 == changed);
        assert!(path.with_file_name("v.kchest.bak").exists());
        assert!(!path.with_file_name("v.kchest.tmp").exists());
    }

    #[test]
    fn wrong_password_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kchest");
        VaultFile::create(&path, "master", test_params(), &sample()).unwrap();
        assert!(matches!(
            VaultFile::open(&path, "Master"),
            Err(Error::WrongPasswordOrCorrupted)
        ));
    }

    #[test]
    fn any_flipped_byte_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kchest");
        VaultFile::create(&path, "pw", test_params(), &sample()).unwrap();
        let good = fs::read(&path).unwrap();
        // Every header byte, and a spread of ciphertext and tag bytes.
        let positions = (0..HEADER_LEN).chain((HEADER_LEN..good.len()).step_by(97));
        for i in positions.chain([good.len() - 1]) {
            let mut bad = good.clone();
            bad[i] ^= 0x01;
            fs::write(&path, &bad).unwrap();
            assert!(
                VaultFile::open(&path, "pw").is_err(),
                "byte {i} not covered"
            );
        }
    }

    #[test]
    fn plaintext_is_not_in_the_file_and_size_is_padded() {
        let key = Zeroizing::new([5; 32]);
        let bytes = encrypt(&key, &[1; SALT_LEN], &test_params(), &sample()).unwrap();
        assert_eq!((bytes.len() - HEADER_LEN - TAG_LEN) % PAD_TO, 0);
        let needle = b"correct horse";
        assert!(!bytes.windows(needle.len()).any(|w| w == needle));
        // Nonces are fresh: the same vault encrypts differently each time.
        let again = encrypt(&key, &[1; SALT_LEN], &test_params(), &sample()).unwrap();
        assert_ne!(bytes, again);
    }

    #[test]
    fn change_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kchest");
        let vault = sample();
        let mut file = VaultFile::create(&path, "old", test_params(), &vault).unwrap();
        assert!(file.verify_password("old").unwrap());
        assert!(!file.verify_password("new").unwrap());
        let stronger = KdfParams {
            iterations: 2,
            ..test_params()
        };
        file.change_password("new", stronger, &vault).unwrap();
        assert!(VaultFile::open(&path, "old").is_err());
        let bak = path.with_file_name("v.kchest.bak");
        assert!(
            VaultFile::open(&bak, "old").is_err(),
            "backup keeps the old key"
        );
        assert!(VaultFile::open(&bak, "new").is_ok());
        let (f, v) = VaultFile::open(&path, "new").unwrap();
        assert!(v == vault);
        assert_eq!(f.kdf(), stronger);
        assert!(file.verify_password("new").unwrap());
    }
}
