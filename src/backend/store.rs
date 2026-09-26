//! Disk cache for library data and cover thumbnails.
//!
//! Anything already downloaded is reused: a playlist whose `snapshot_id` did not
//! change is never downloaded again, albums never change, and covers are kept as
//! small JPEG files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::write_atomic;

#[derive(Clone)]
pub struct Store {
    data: PathBuf,
    images: PathBuf,
}

/// Upper bound for the library cache; oldest files are removed beyond it.
const DATA_LIMIT: u64 = 64 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;

impl Store {
    pub fn new(data: PathBuf, images: PathBuf) -> Self {
        let _ = fs::create_dir_all(&data);
        let _ = fs::create_dir_all(&images);
        Self { data, images }
    }

    fn data_path(&self, key: &str) -> PathBuf {
        self.data.join(format!("{}.json", sanitize(key)))
    }

    pub fn load<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let bytes = fs::read(self.data_path(key)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save<T: Serialize>(&self, key: &str, value: &T) {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&self.data_path(key), &bytes) {
                    log::warn!("cache write failed for {key}: {e}");
                }
            }
            Err(e) => log::warn!("cache serialization failed for {key}: {e}"),
        }
    }

    pub fn image_path(&self, url: &str) -> PathBuf {
        // Cover URLs end with a unique hash: https://i.scdn.co/image/ab67616d0000485…
        let name = url.rsplit('/').next().unwrap_or(url);
        self.images.join(format!("{}.jpg", sanitize(name)))
    }

    pub fn load_image(&self, url: &str) -> Option<Vec<u8>> {
        fs::read(self.image_path(url)).ok()
    }

    pub fn save_image(&self, url: &str, bytes: &[u8]) {
        if let Err(e) = write_atomic(&self.image_path(url), bytes) {
            log::warn!("cover cache write failed: {e}");
        }
    }

    /// Removes the oldest files when a cache grows too large.
    pub fn prune(&self) {
        prune_dir(&self.data, DATA_LIMIT);
        prune_dir(&self.images, IMAGE_LIMIT);
    }

    pub fn clear(&self) {
        for dir in [&self.data, &self.images] {
            let _ = fs::remove_dir_all(dir);
            let _ = fs::create_dir_all(dir);
        }
    }

    pub fn size(&self) -> u64 {
        dir_size(&self.data) + dir_size(&self.images)
    }
}

fn sanitize(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn prune_dir(dir: &Path, limit: u64) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= limit * 3 / 4 {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("spotilite-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (Store::new(root.join("data"), root.join("img")), root)
    }

    #[test]
    fn roundtrip_and_sanitized_keys() {
        let (store, root) = temp_store("roundtrip");
        store.save("playlist-../../evil", &vec![1, 2, 3]);
        assert_eq!(store.load::<Vec<i32>>("playlist-../../evil"), Some(vec![1, 2, 3]));
        assert!(root.join("data").join("playlist-______evil.json").exists());
        assert_eq!(store.load::<Vec<i32>>("missing"), None);
        let url = "https://i.scdn.co/image/ab67616d00004851abc";
        store.save_image(url, b"jpg");
        assert_eq!(store.load_image(url).as_deref(), Some(&b"jpg"[..]));
        assert!(store.size() > 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_removes_oldest_files_first() {
        let (store, root) = temp_store("prune");
        let dir = root.join("data");
        for i in 0..10 {
            fs::write(dir.join(format!("{i}.json")), vec![0u8; 100]).unwrap();
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i);
            let f = fs::File::options().write(true).open(dir.join(format!("{i}.json"))).unwrap();
            f.set_modified(t).unwrap();
        }
        prune_dir(&dir, 500);
        assert!(!dir.join("0.json").exists());
        assert!(dir.join("9.json").exists());
        assert!(dir_size(&dir) <= 375);
        drop(store);
        let _ = fs::remove_dir_all(root);
    }
}
