//! UI access to (encrypted) attachment files: decrypted, cached images for
//! display and decrypted copies for opening in other applications.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{Image, ImageFormat};
use signal_core::attachments::AttachmentStore;

/// Most decoded images kept in memory.
const CACHE_LIMIT: usize = 64;

#[derive(Default)]
pub struct AttachmentAccess {
    store: Option<AttachmentStore>,
    open_dir: Option<PathBuf>,
    images: RefCell<HashMap<String, Option<Arc<Image>>>>,
}

impl AttachmentAccess {
    pub fn new(store: AttachmentStore, open_dir: PathBuf) -> Self {
        // decrypted copies from a previous session must not linger
        let _ = std::fs::remove_dir_all(&open_dir);
        Self {
            store: Some(store),
            open_dir: Some(open_dir),
            images: RefCell::default(),
        }
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        match &self.store {
            Some(store) => store.read(path),
            None => std::fs::read(path),
        }
    }

    /// The decoded image for an attachment file, decrypting on first use.
    pub fn image(&self, path: &str, content_type: &str) -> Option<Arc<Image>> {
        if let Some(cached) = self.images.borrow().get(path) {
            return cached.clone();
        }
        let format = ImageFormat::from_mime_type(content_type).or_else(|| {
            Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|e| ImageFormat::from_mime_type(&format!("image/{}", e.to_lowercase())))
        });
        let image = match (format, self.read(Path::new(path))) {
            (Some(format), Ok(bytes)) => Some(Arc::new(Image::from_bytes(format, bytes))),
            (_, Err(e)) => {
                tracing::warn!(%e, "failed to read attachment");
                None
            }
            (None, _) => None,
        };
        let mut images = self.images.borrow_mut();
        if images.len() >= CACHE_LIMIT {
            images.clear();
        }
        images.insert(path.to_string(), image.clone());
        image
    }

    /// Writes a decrypted copy of an attachment to `dest` (Save As).
    pub fn export_to(&self, path: &str, dest: &Path) -> std::io::Result<()> {
        std::fs::write(dest, self.read(Path::new(path))?)
    }

    /// Deletes an attachment file if (and only if) it lives in our store.
    pub fn remove_stored_file(&self, path: &str) {
        if let Some(store) = &self.store
            && Path::new(path).starts_with(store.dir())
        {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Path suitable for `open_with_system`: a decrypted copy for stored
    /// attachments, the original for plaintext files.
    pub fn path_for_open(&self, path: &str, file_name: &str) -> std::io::Result<PathBuf> {
        match (&self.store, &self.open_dir) {
            (Some(store), Some(dir)) if path.ends_with(".sga") => {
                store.export_for_open(Path::new(path), file_name, dir)
            }
            _ => Ok(PathBuf::from(path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1x1 transparent PNG
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9C, 0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0,
        0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn decrypts_and_caches_images() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path().join("a"), b"k");
        let path = store.write("img", PNG).unwrap();
        let access = AttachmentAccess::new(store, dir.path().join("open"));
        let p = path.to_string_lossy().to_string();
        let img = access.image(&p, "image/png").expect("decoded");
        assert_eq!(img.bytes(), PNG);
        std::fs::remove_file(&path).unwrap();
        assert!(access.image(&p, "image/png").is_some(), "served from cache");
    }

    #[test]
    fn opening_writes_a_decrypted_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path().join("a"), b"k");
        let path = store.write("doc", b"%PDF").unwrap();
        let access = AttachmentAccess::new(store, dir.path().join("open"));
        let out = access
            .path_for_open(&path.to_string_lossy(), "report.pdf")
            .unwrap();
        assert!(out.starts_with(dir.path().join("open")));
        assert_eq!(std::fs::read(out).unwrap(), b"%PDF");
        // plaintext originals are opened in place
        assert_eq!(
            access.path_for_open("/x/y.txt", "y.txt").unwrap(),
            PathBuf::from("/x/y.txt")
        );
    }
}
