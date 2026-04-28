//! Disk-backed LRU cache for album art.
//!
//! Images are stored under SHA-256-hashed filenames. Entries and LRU order
//! persist to a JSON sidecar so cache state survives restarts.
//!
//! Entries can be marked as `pinned`, in which case they are skipped by
//! LRU eviction and survive a `flush()`. Used to keep album art for
//! user-downloaded tracks resident while online browsing churns the rest
//! of the cache.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const META_FILENAME: &str = "image_cache_meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheMeta {
    entries: HashMap<String, CacheEntry>,
    access_order: Vec<String>,
}

pub struct ImageCache {
    cache_dir: PathBuf,
    entries: HashMap<String, CacheEntry>,
    /// Oldest first.
    access_order: Vec<String>,
    limit_bytes: u64,
}

impl ImageCache {
    /// Load an existing cache from disk, or return an empty one.
    pub fn load(cache_dir: PathBuf, limit_bytes: u64) -> Self {
        let meta_path = cache_dir.join(META_FILENAME);
        let meta = fs::read(&meta_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<CacheMeta>(&data).ok())
            .unwrap_or_default();

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

    /// Deterministic cache key from thumb path and size.
    fn cache_key(thumb: &str, size: u32) -> String {
        let input = format!("{}_{}", thumb, size);
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)
    }

    /// Look up a cached image. Marks the entry as most-recently-used on hit.
    pub fn get(&mut self, thumb: &str, size: u32) -> Option<PathBuf> {
        let key = Self::cache_key(thumb, size);
        let entry = self.entries.get(&key)?;

        if !entry.path.exists() {
            self.entries.remove(&key);
            self.access_order.retain(|k| k != &key);
            return None;
        }

        self.access_order.retain(|k| k != &key);
        self.access_order.push(key);

        Some(entry.path.clone())
    }

    /// Store image data in the cache. Returns the path to the written file.
    pub fn insert(&mut self, thumb: &str, size: u32, data: &[u8]) -> Result<PathBuf, String> {
        self.insert_inner(thumb, size, data, false)
    }

    /// Like `insert`, but marks the entry as pinned so it survives LRU
    /// eviction and `flush()`. If the key already exists, its pin flag is
    /// promoted to true (pin is sticky).
    pub fn insert_pinned(
        &mut self,
        thumb: &str,
        size: u32,
        data: &[u8],
    ) -> Result<PathBuf, String> {
        self.insert_inner(thumb, size, data, true)
    }

    fn insert_inner(
        &mut self,
        thumb: &str,
        size: u32,
        data: &[u8],
        pinned: bool,
    ) -> Result<PathBuf, String> {
        let key = Self::cache_key(thumb, size);

        // Concurrent download may already have populated this entry.
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.path.exists() {
                let path = entry.path.clone();
                let promoted = pinned && !entry.pinned;
                if promoted {
                    entry.pinned = true;
                }
                if promoted {
                    self.save_meta();
                }
                return Ok(path);
            }
        }

        fs::create_dir_all(&self.cache_dir).map_err(|e| e.to_string())?;

        let file_path = self.cache_dir.join(format!("{}.jpg", key));
        atomic_write(&file_path, data).map_err(|e| e.to_string())?;

        let file_size = data.len() as u64;

        self.access_order.retain(|k| k != &key);
        self.entries.insert(
            key.clone(),
            CacheEntry {
                path: file_path.clone(),
                size: file_size,
                pinned,
            },
        );
        self.access_order.push(key);

        self.evict_if_needed();
        self.save_meta();

        Ok(file_path)
    }

    /// Pin a single existing entry. No-op if the entry is missing or
    /// already pinned. Used when `warm_art_cache` finds the art already
    /// on disk and only needs to flip its flag.
    pub fn pin(&mut self, thumb: &str, size: u32) {
        let key = Self::cache_key(thumb, size);
        if let Some(entry) = self.entries.get_mut(&key) {
            if !entry.pinned {
                entry.pinned = true;
                self.save_meta();
            }
        }
    }

    /// Reconcile the pinned set against `thumbs`: any cached entry whose
    /// `(thumb, size)` matches one of the supplied thumbs at one of the
    /// supplied sizes becomes pinned; any entry currently pinned but not
    /// in the desired set is unpinned (and becomes evictable). Used after
    /// download removals and at startup to restore pin state from the
    /// downloads table.
    pub fn set_pinned_thumbs(&mut self, thumbs: &HashSet<String>, sizes: &[u32]) {
        let mut desired: HashSet<String> = HashSet::with_capacity(thumbs.len() * sizes.len());
        for thumb in thumbs {
            for &sz in sizes {
                desired.insert(Self::cache_key(thumb, sz));
            }
        }
        let mut changed = false;
        for (key, entry) in self.entries.iter_mut() {
            let should_pin = desired.contains(key);
            if entry.pinned != should_pin {
                entry.pinned = should_pin;
                changed = true;
            }
        }
        if changed {
            self.save_meta();
        }
    }

    /// Delete all cached files and metadata. Pinned entries are kept so a
    /// manual flush doesn't undermine offline downloads.
    pub fn flush(&mut self) -> Result<(), String> {
        self.entries.retain(|_, entry| {
            if entry.pinned {
                true
            } else {
                let _ = fs::remove_file(&entry.path);
                false
            }
        });
        let kept: HashSet<&String> = self.entries.keys().collect();
        self.access_order.retain(|k| kept.contains(k));
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

    /// Total size of pinned entries (those that survive LRU eviction
    /// and `flush()`). Lets the UI explain why a flush doesn't take the
    /// reported cache size to zero.
    pub fn pinned_size(&self) -> u64 {
        self.entries
            .values()
            .filter(|e| e.pinned)
            .map(|e| e.size)
            .sum()
    }

    pub fn pinned_count(&self) -> usize {
        self.entries.values().filter(|e| e.pinned).count()
    }

    fn evict_if_needed(&mut self) {
        while self.total_size() > self.limit_bytes {
            // Find the oldest *unpinned* entry. If everything left is
            // pinned, accept that the cache exceeds its limit — pinned
            // entries are load-bearing for offline playback.
            let mut victim_idx: Option<usize> = None;
            for (i, k) in self.access_order.iter().enumerate() {
                let unpinned = self.entries.get(k).map(|e| !e.pinned).unwrap_or(true);
                if unpinned {
                    victim_idx = Some(i);
                    break;
                }
            }
            let Some(idx) = victim_idx else {
                break;
            };
            let key = self.access_order.remove(idx);
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
            let _ = atomic_write(&self.cache_dir.join(META_FILENAME), &data);
        }
    }
}

/// Stage data into a sibling `.tmp` file, fsync, then rename atomically.
/// Without this, an iOS suspension or crash mid-write could leave a
/// truncated file on disk that subsequent loads would treat as valid —
/// for the meta sidecar that means losing the entire LRU index.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let tmp_path = path.with_extension({
        let mut e = path
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        e.push_str(".tmp");
        e
    });
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, path)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Shim module so `cache_key` can use the local `hex_encode` without adding a dep.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        super::hex_encode(bytes.as_ref())
    }
}

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
        let (mut cache, _dir) = temp_cache(30);
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap();
        cache.insert("/c", 100, b"cccccccccc").unwrap();
        assert_eq!(cache.entry_count(), 3);

        cache.insert("/d", 100, b"dddddddddd").unwrap();
        assert!(cache.get("/a", 100).is_none());
        assert!(cache.get("/d", 100).is_some());
    }

    #[test]
    fn test_touch_on_get_prevents_eviction() {
        let (mut cache, _dir) = temp_cache(30);
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap();
        cache.insert("/c", 100, b"cccccccccc").unwrap();

        // Touch /a so it becomes MRU.
        cache.get("/a", 100);

        cache.insert("/d", 100, b"dddddddddd").unwrap();
        assert!(cache.get("/a", 100).is_some());
        assert!(cache.get("/b", 100).is_none());
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

        fs::remove_file(&file_path).unwrap();

        let mut cache = ImageCache::load(path, 1_000_000);
        assert!(cache.get("/thumb/1", 300).is_none());
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn test_set_limit_triggers_eviction() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap();
        assert_eq!(cache.entry_count(), 2);

        cache.set_limit(15);
        assert_eq!(cache.entry_count(), 1);
    }

    #[test]
    fn test_pinned_entries_survive_lru_pressure() {
        let (mut cache, _dir) = temp_cache(30);
        cache.insert_pinned("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert("/b", 100, b"bbbbbbbbbb").unwrap();
        cache.insert("/c", 100, b"cccccccccc").unwrap();
        cache.insert("/d", 100, b"dddddddddd").unwrap();
        // /a is pinned and oldest, so /b should have been evicted instead.
        assert!(cache.get("/a", 100).is_some());
        assert!(cache.get("/b", 100).is_none());
    }

    #[test]
    fn test_pinned_entries_can_exceed_limit() {
        let (mut cache, _dir) = temp_cache(15);
        cache.insert_pinned("/a", 100, b"aaaaaaaaaa").unwrap();
        cache.insert_pinned("/b", 100, b"bbbbbbbbbb").unwrap();
        // Both pinned, total=20 > 15 — both must remain.
        assert_eq!(cache.entry_count(), 2);
        assert!(cache.total_size() > cache.limit_bytes);
    }

    #[test]
    fn test_set_pinned_thumbs_pins_and_unpins() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"data").unwrap();
        cache.insert("/b", 100, b"data").unwrap();
        cache.insert_pinned("/c", 100, b"data").unwrap();

        let mut want = HashSet::new();
        want.insert("/a".to_string());
        cache.set_pinned_thumbs(&want, &[100]);

        // /a now pinned, /c was pinned but now unpinned.
        assert!(cache.entries.get(&ImageCache::cache_key("/a", 100)).unwrap().pinned);
        assert!(!cache.entries.get(&ImageCache::cache_key("/c", 100)).unwrap().pinned);
    }

    #[test]
    fn test_flush_keeps_pinned() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"data").unwrap();
        cache.insert_pinned("/b", 100, b"data").unwrap();
        cache.flush().unwrap();
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.get("/b", 100).is_some());
        assert!(cache.get("/a", 100).is_none());
    }

    #[test]
    fn test_pinned_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let mut cache = ImageCache::load(path.clone(), 1_000_000);
            cache.insert_pinned("/a", 100, b"data").unwrap();
        }
        let cache = ImageCache::load(path, 1_000_000);
        let entry = cache.entries.get(&ImageCache::cache_key("/a", 100)).unwrap();
        assert!(entry.pinned);
    }

    #[test]
    fn test_insert_promotes_existing_to_pinned() {
        let (mut cache, _dir) = temp_cache(1_000_000);
        cache.insert("/a", 100, b"data").unwrap();
        cache.insert_pinned("/a", 100, b"data").unwrap();
        let entry = cache.entries.get(&ImageCache::cache_key("/a", 100)).unwrap();
        assert!(entry.pinned);
    }
}
