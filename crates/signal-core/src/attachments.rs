//! Encrypted local attachment store.
//!
//! Downloaded attachments are written as `MAGIC | nonce (12) | AES-256-GCM
//! ciphertext+tag`, keyed by HKDF-SHA256 of the database key. Files without
//! the magic header (e.g. the user's own outgoing files referenced in place)
//! are read back unchanged.

use std::io;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

const MAGIC: &[u8; 4] = b"SGA1";
const NONCE_LEN: usize = 12;

#[derive(Clone)]
pub struct AttachmentStore {
    dir: PathBuf,
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for AttachmentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentStore")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

impl AttachmentStore {
    /// `secret` is the database key; a dedicated key is derived from it.
    pub fn new(dir: impl Into<PathBuf>, secret: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(b"signal-gpui"), secret);
        let mut key = [0u8; 32];
        hk.expand(b"attachment-store-v1", &mut key)
            .expect("32 bytes is a valid HKDF length");
        Self {
            dir: dir.into(),
            cipher: Aes256Gcm::new(&key.into()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| invalid("encryption failed"))?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Decrypts store-encrypted bytes; other data is returned as-is.
    pub fn decrypt(&self, data: Vec<u8>) -> io::Result<Vec<u8>> {
        let Some(rest) = data.strip_prefix(MAGIC.as_slice()) else {
            return Ok(data);
        };
        if rest.len() < NONCE_LEN {
            return Err(invalid("truncated attachment"));
        }
        let (nonce, ct) = rest.split_at(NONCE_LEN);
        self.cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| invalid("attachment failed authentication (wrong key or corrupted)"))
    }

    /// Encrypts and writes `bytes` as `name` (atomically) inside the store.
    pub fn write(&self, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{name}.sga"));
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, self.encrypt(bytes)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// Reads (and decrypts if needed) an attachment file.
    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.decrypt(std::fs::read(path)?)
    }

    /// Decrypts an attachment into `dir` under its original name so it can be
    /// opened by another application. Returns the plaintext path.
    pub fn export_for_open(&self, path: &Path, file_name: &str, dir: &Path) -> io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let safe: String = file_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe = safe.trim_start_matches('.');
        let out = dir.join(if safe.is_empty() { "attachment" } else { safe });
        std::fs::write(&out, self.read(path)?)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_ciphertext_hides_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path(), b"db key");
        let path = store.write("1-0-photo.jpg", b"very secret pixels").unwrap();
        assert!(path.extension().is_some_and(|e| e == "sga"));
        let raw = std::fs::read(&path).unwrap();
        assert!(raw.starts_with(MAGIC));
        assert!(!raw.windows(6).any(|w| w == b"secret"));
        assert_eq!(store.read(&path).unwrap(), b"very secret pixels");
        // same plaintext, different nonce
        assert_ne!(store.encrypt(b"x").unwrap(), store.encrypt(b"x").unwrap());
    }

    #[test]
    fn wrong_key_and_tampering_are_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path(), b"key one");
        let path = store.write("a", b"data").unwrap();
        let other = AttachmentStore::new(dir.path(), b"key two");
        assert!(other.read(&path).is_err());
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert!(store.decrypt(raw).is_err());
        assert!(store.decrypt(b"SGA1short".to_vec()).is_err());
    }

    #[test]
    fn plaintext_files_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path(), b"k");
        let p = dir.path().join("mine.txt");
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(store.read(&p).unwrap(), b"hello");
    }

    #[test]
    fn export_sanitizes_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path().join("store"), b"k");
        let p = store.write("x", b"doc").unwrap();
        let out = store
            .export_for_open(&p, "../../evil.pdf", &dir.path().join("open"))
            .unwrap();
        assert_eq!(out.parent().unwrap(), dir.path().join("open"));
        assert_eq!(std::fs::read(out).unwrap(), b"doc");
    }
}
