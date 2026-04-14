//! LRU download cache tracking cached audio file metadata. The caller
//! handles file I/O; `evict_if_needed` returns paths to delete.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct DownloadCache {
    entries: HashMap<String, PathBuf>,
    sizes: HashMap<String, u64>,
    /// Oldest first.
    access_order: Vec<String>,
    pub(crate) limit_bytes: u64,
}

impl DownloadCache {
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            entries: HashMap::new(),
            sizes: HashMap::new(),
            access_order: Vec::new(),
            limit_bytes,
        }
    }

    /// Get the cached file path for a track, if present.
    pub fn get(&self, track_id: &str) -> Option<&Path> {
        self.entries.get(track_id).map(|p| p.as_path())
    }

    /// Insert a cached file entry.
    pub fn insert(&mut self, track_id: String, path: PathBuf, size: u64) {
        self.access_order.retain(|k| k != &track_id);
        self.entries.insert(track_id.clone(), path);
        self.sizes.insert(track_id.clone(), size);
        self.access_order.push(track_id);
    }

    /// Touch a cache entry, moving it to the most-recently-used position.
    pub fn touch(&mut self, track_id: &str) {
        if self.entries.contains_key(track_id) {
            self.access_order.retain(|k| k != track_id);
            self.access_order.push(track_id.to_string());
        }
    }

    /// Evict oldest entries until total size is within the limit. Never
    /// evicts the currently playing track. Returns paths to delete from disk.
    pub fn evict_if_needed(&mut self, current_track_id: Option<&str>) -> Vec<PathBuf> {
        let mut evicted = Vec::new();

        while self.total_size() > self.limit_bytes && !self.access_order.is_empty() {
            let idx = self
                .access_order
                .iter()
                .position(|k| current_track_id.is_none_or(|c| k != c));

            if let Some(idx) = idx {
                let key = self.access_order.remove(idx);
                if let Some(path) = self.entries.remove(&key) {
                    evicted.push(path);
                }
                self.sizes.remove(&key);
            } else {
                break;
            }
        }

        evicted
    }

    pub fn total_size(&self) -> u64 {
        self.sizes.values().sum()
    }

    /// Remove a specific entry. Returns the path if it was present.
    pub fn remove(&mut self, track_id: &str) -> Option<PathBuf> {
        self.access_order.retain(|k| k != track_id);
        self.sizes.remove(track_id);
        self.entries.remove(track_id)
    }

    /// Clear all entries. Returns all paths for disk cleanup.
    pub fn clear(&mut self) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = self.entries.drain().map(|(_, p)| p).collect();
        self.sizes.clear();
        self.access_order.clear();
        paths
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = DownloadCache::new(1_000_000);
        assert!(cache.get("1").is_none());

        cache.insert("1".into(), PathBuf::from("/tmp/1.flac"), 500_000);
        assert_eq!(cache.get("1"), Some(Path::new("/tmp/1.flac")));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 500_000);
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = DownloadCache::new(1000);
        cache.insert("a".into(), PathBuf::from("/a"), 400);
        cache.insert("b".into(), PathBuf::from("/b"), 400);
        cache.insert("c".into(), PathBuf::from("/c"), 400);

        let evicted = cache.evict_if_needed(None);
        assert!(evicted.contains(&PathBuf::from("/a")));
        assert!(cache.get("a").is_none());
        assert_eq!(cache.total_size(), 800);
    }

    #[test]
    fn test_cache_touch_updates_order() {
        let mut cache = DownloadCache::new(1000);
        cache.insert("a".into(), PathBuf::from("/a"), 400);
        cache.insert("b".into(), PathBuf::from("/b"), 400);
        cache.insert("c".into(), PathBuf::from("/c"), 400);
        cache.touch("a");

        let evicted = cache.evict_if_needed(None);
        assert!(evicted.contains(&PathBuf::from("/b")));
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_cache_protects_current_track() {
        let mut cache = DownloadCache::new(500);
        cache.insert("playing".into(), PathBuf::from("/playing"), 400);
        cache.insert("next".into(), PathBuf::from("/next"), 400);

        let evicted = cache.evict_if_needed(Some("playing"));
        assert!(evicted.contains(&PathBuf::from("/next")));
        assert!(cache.get("playing").is_some());
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/a"), 100);
        cache.insert("b".into(), PathBuf::from("/b"), 200);

        let paths = cache.clear();
        assert_eq!(paths.len(), 2);
        assert!(cache.is_empty());
        assert_eq!(cache.total_size(), 0);
    }

    #[test]
    fn test_cache_remove() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/a"), 100);
        cache.insert("b".into(), PathBuf::from("/b"), 200);

        let removed = cache.remove("a");
        assert_eq!(removed, Some(PathBuf::from("/a")));
        assert!(cache.get("a").is_none());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 200);
    }

    #[test]
    fn test_cache_update_existing_entry() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/old"), 100);
        cache.insert("a".into(), PathBuf::from("/new"), 200);

        assert_eq!(cache.get("a"), Some(Path::new("/new")));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 200);
    }
}
