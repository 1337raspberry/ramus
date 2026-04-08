//! LRU disk-based image cache for album art.
//!
//! Stores downloaded images on disk with SHA-256 hashed filenames.
//! Metadata (entries + LRU order) is persisted to a JSON sidecar file
//! so cache state survives app restarts.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const META_FILENAME: &str = "image_cache_meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheMeta {
    entries: HashMap<String, CacheEntry>,
    access_order: Vec<String>,
}

pub struct ImageCache {
    cache_dir: PathBuf,
    entries: HashMap<String, CacheEntry>,
    access_order: Vec<String>, // oldest first
    limit_bytes: u64,
}

impl ImageCache {
    /// Load an existing cache from disk, or create an empty one.
    pub fn load(cache_dir: PathBuf, limit_bytes: u64) -> Self {
        let meta_path = cache_dir.join(META_FILENAME);
        let meta = fs::read(&meta_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<CacheMeta>(&data).ok())
            .unwrap_or_default();

        // Prune stale entries whose files no longer exist
        let mut entries = HashMap::new();
        let mut access_order = Vec::new();
        for key in &meta.access_order {
            if let Some(entry) = meta.entries.get(key) {
                if entry.path.exists() {
                    entries.insert(key.clone(), entry.clone());
                    access_order.push(key.clone());
                }
            }
        }

        let mut cache = Self {
            cache_dir,
            entries,
            access_order,
            limit_bytes,
        };
        cache.evict_if_needed();
        cache
    }

    /// Build a deterministic cache key from thumb path + size.
    fn cache_key(thumb: &str, size: u32) -> String {
        let input = format!("{}_{}", thumb, size);
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)
    }

    /// Look up a cached image. Returns the file path on hit, touching
    /// the entry to mark it as recently used.
    pub fn get(&mut self, thumb: &str, size: u32) -> Option<PathBuf> {
        let key = Self::cache_key(thumb, size);
        let entry = self.entries.get(&key)?;

        if !entry.path.exists() {
            // File was deleted externally — clean up
            self.entries.remove(&key);
            self.access_order.retain(|k| k != &key);
            return None;
        }

        // Touch: move to MRU
        self.access_order.retain(|k| k != &key);
        self.access_order.push(key);

        Some(entry.path.clone())
    }

    /// Store image data in the cache. Returns the path to the written file.
    pub fn insert(&mut self, thumb: &str, size: u32, data: &[u8]) -> Result<PathBuf, String> {
        let key = Self::cache_key(thumb, size);

        // Already cached (race with concurrent download)
        if let Some(entry) = self.entries.get(&key) {
            if entry.path.exists() {
                return Ok(entry.path.clone());
            }
        }

        fs::create_dir_all(&self.cache_dir).map_err(|e| e.to_string())?;

        let file_path = self.cache_dir.join(format!("{}.jpg", key));
        fs::write(&file_path, data).map_err(|e| e.to_string())?;

        let file_size = data.len() as u64;

        // Remove old entry if updating
        self.access_order.retain(|k| k != &key);
        self.entries.insert(
            key.clone(),
            CacheEntry {
                path: file_path.clone(),
                size: file_size,
            },
        );
        self.access_order.push(key);

        self.evict_if_needed();
        self.save_meta();

        Ok(file_path)
    }

    /// Delete all cached files and metadata.
    pub fn flush(&mut self) -> Result<(), String> {
        for entry in self.entries.values() {
            let _ = fs::remove_file(&entry.path);
        }
        self.entries.clear();
        self.access_order.clear();
        self.save_meta();
        Ok(())
    }

    pub fn set_limit(&mut self, limit_bytes: u64) {
        self.limit_bytes = limit_bytes;
        self.evict_if_needed();
        self.save_meta();
    }

    pub fn total_size(&self) -> u64 {
        self.entries.values().map(|e| e.size).sum()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    // -- internals --

    fn evict_if_needed(&mut self) {
        while self.total_size() > self.limit_bytes && !self.access_order.is_empty() {
            let key = self.access_order.remove(0);
            if let Some(entry) = self.entries.remove(&key) {
                let _ = fs::remove_file(&entry.path);
            }
        }
    }

    fn save_meta(&self) {
        let meta = CacheMeta {
            entries: self.entries.clone(),
            access_order: self.access_order.clone(),
        };
        if let Ok(data) = serde_json::to_vec(&meta) {
            let _ = fs::write(self.cache_dir.join(META_FILENAME), data);
        }
    }
}

// ---------------------------------------------------------------------------
// hex encoding (avoid adding a dependency for this)
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// Use our local hex_encode in cache_key
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        super::hex_encode(bytes.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_cache(limit: u64) -> (ImageCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::load(dir.path().to_path_buf(), limit);
        (cache, dir)
    }

    #[test]
    fn test_insert_and_get() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        let path = cache.insert("/thumb/1", 300, b"fake-image-data").unwrap();
        assert!(path.exists());
        assert_eq!(cache.get("/thumb/1", 300), Some(path));
    }

    #[test]
    fn test_different_sizes_different_keys() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        let p1 = cache.insert("/thumb/1", 300, b"small").unwrap();
        let p2 = cache.insert("/thumb/1", 600, b"large").unwrap();
        assert_ne!(p1, p2);
        assert_eq!(cache.entry_count(), 2);
    }

    #[test]
    fn test_lru_eviction() {
        let (mut cache, _dir) = temp_cache(30); // tiny limit
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap(); // 10 bytes
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap(); // 10 bytes
        cache.insert("/c", 100, b"cccccccccc").unwrap(); // 10 bytes
        // 30 bytes total = at limit
        assert_eq!(cache.entry_count(), 3);

        cache.insert("/d", 100, b"dddddddddd").unwrap(); // 40 > 30 → evict oldest
        assert!(cache.get("/a", 100).is_none()); // evicted
        assert!(cache.get("/d", 100).is_some());
    }

    #[test]
    fn test_touch_on_get_prevents_eviction() {
        let (mut cache, _dir) = temp_cache(30);
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap();
        cache.insert("/c", 100, b"cccccccccc").unwrap();

        // Touch /a so it's MRU
        cache.get("/a", 100);

        cache.insert("/d", 100, b"dddddddddd").unwrap();
        assert!(cache.get("/a", 100).is_some()); // kept (was MRU)
        assert!(cache.get("/b", 100).is_none()); // evicted (was oldest)
    }

    #[test]
    fn test_flush_clears_all() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"data").unwrap();
        cache.insert("/b", 100, b"data").unwrap();
        assert_eq!(cache.entry_count(), 2);

        cache.flush().unwrap();
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.total_size(), 0);
    }

    #[test]
    fn test_metadata_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        {
            let mut cache = ImageCache::load(path.clone(), 1_000_000);
            cache.insert("/thumb/1", 300, b"image-data").unwrap();
            assert_eq!(cache.entry_count(), 1);
        }

        // Reload from disk
        let mut cache = ImageCache::load(path, 1_000_000);
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.get("/thumb/1", 300).is_some());
    }

    #[test]
    fn test_stale_entry_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let file_path;
        {
            let mut cache = ImageCache::load(path.clone(), 1_000_000);
            file_path = cache.insert("/thumb/1", 300, b"data").unwrap();
        }

        // Delete the file externally
        fs::remove_file(&file_path).unwrap();

        let mut cache = ImageCache::load(path, 1_000_000);
        assert!(cache.get("/thumb/1", 300).is_none());
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn test_set_limit_triggers_eviction() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap(); // 10 bytes
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap(); // 10 bytes
        assert_eq!(cache.entry_count(), 2);

        cache.set_limit(15); // only room for ~1 entry
        assert_eq!(cache.entry_count(), 1);
    }
}
