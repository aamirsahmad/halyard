//! On-disk layout of the client's data directory and database key handling.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataPaths {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct KeyFile {
    key: String,
}

impl DataPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// presage/libsignal protocol state (identity keys, sessions…).
    pub fn protocol_db(&self) -> PathBuf {
        self.root.join("protocol.db3")
    }

    /// Conversations and messages index.
    pub fn app_db(&self) -> PathBuf {
        self.root.join("messages.db3")
    }

    pub fn attachments_dir(&self) -> PathBuf {
        self.root.join("attachments")
    }

    /// Encrypted store for downloaded attachments.
    pub fn attachment_store(&self, db_key: &str) -> signal_core::attachments::AttachmentStore {
        signal_core::attachments::AttachmentStore::new(self.attachments_dir(), db_key.as_bytes())
    }

    /// Scratch directory for attachments decrypted to be opened externally.
    /// Cleared on every start.
    pub fn open_cache_dir(&self) -> PathBuf {
        self.root.join("open-cache")
    }

    pub fn key_file(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// Creates the directory tree, readable only by the current user.
    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(self.attachments_dir())?;
        restrict(&self.root, 0o700)?;
        restrict(&self.attachments_dir(), 0o700)?;
        Ok(())
    }

    /// Loads the database encryption key, generating one on first launch.
    ///
    /// Like Signal Desktop, the key is kept in a user-only readable config
    /// file next to the encrypted databases; the databases are useless
    /// without it.
    pub fn load_or_create_db_key(&self) -> io::Result<String> {
        self.ensure()?;
        let path = self.key_file();
        match fs::read_to_string(&path) {
            Ok(s) => {
                let kf: KeyFile = serde_json::from_str(&s)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if kf.key.len() != 64 || !kf.key.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "malformed database key",
                    ));
                }
                Ok(kf.key)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut bytes = [0u8; 32];
                rand::fill(&mut bytes);
                let key = hex::encode(bytes);
                let tmp = path.with_extension("json.tmp");
                fs::write(
                    &tmp,
                    serde_json::to_vec_pretty(&KeyFile { key: key.clone() })?,
                )?;
                restrict(&tmp, 0o600)?;
                fs::rename(&tmp, &path)?;
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_: &Path, _: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let p = DataPaths::new("/data");
        assert_eq!(p.protocol_db(), PathBuf::from("/data/protocol.db3"));
        assert_eq!(p.app_db(), PathBuf::from("/data/messages.db3"));
        assert_eq!(p.attachments_dir(), PathBuf::from("/data/attachments"));
    }

    #[test]
    fn key_is_created_once_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = DataPaths::new(dir.path().join("signal"));
        let k1 = p.load_or_create_db_key().unwrap();
        let k2 = p.load_or_create_db_key().unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(p.key_file()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let mode = fs::metadata(p.root()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn corrupted_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = DataPaths::new(dir.path());
        p.ensure().unwrap();
        fs::write(p.key_file(), r#"{"key":"nothex"}"#).unwrap();
        assert!(p.load_or_create_db_key().is_err());
        fs::write(p.key_file(), "garbage").unwrap();
        assert!(p.load_or_create_db_key().is_err());
    }
}
