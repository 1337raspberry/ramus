//! On-disk audio: the permanent-download registry, the LRU prefetch cache,
//! and the lookahead-window queries the prefetch worker drives itself from.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::playback::download_cache::DownloadCache;
use crate::playback::transcode;

use super::adaptive::effective_stream_policy;
use super::AudioPlayer;

/// A lookahead-window track whose audio is already on disk, paired with
/// everything the prefetch worker needs to warm its ancillary caches
/// (waveform sidecar + hero art). Only tracks with secured audio are
/// returned — we never warm the extras for something we can't play offline.
#[derive(Debug, Clone)]
pub struct WarmTarget {
    pub rating_key: String,
    pub audio_path: PathBuf,
    pub thumb: Option<String>,
}

impl AudioPlayer {
    /// Register a downloaded file as permanently cached. Takes priority
    /// over the LRU prefetch cache in `resolve_url`.
    pub fn register_persistent_download(&self, rating_key: String, path: PathBuf) {
        self.persistent_cache.write().insert(rating_key, path);
    }

    /// Unregister a downloaded file (e.g. user removed it from the downloads panel).
    pub fn unregister_persistent_download(&self, rating_key: &str) {
        self.persistent_cache.write().remove(rating_key);
    }

    /// Replace the entire persistent cache. Called once at app startup
    /// after loading the `downloads` table.
    pub fn rehydrate_persistent_cache(&self, entries: HashMap<String, PathBuf>) {
        *self.persistent_cache.write() = entries;
    }

    /// Whether a rating key has a permanent download on disk.
    pub fn has_persistent_download(&self, rating_key: &str) -> bool {
        self.persistent_cache.read().contains_key(rating_key)
    }

    /// Snapshot of all persistent download paths. Used by the downloads panel.
    pub fn persistent_download_paths(&self) -> HashMap<String, PathBuf> {
        self.persistent_cache.read().clone()
    }

    /// Access the download cache under the player lock.
    pub fn with_cache<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DownloadCache) -> R,
    {
        let mut inner = self.inner.lock();
        f(&mut inner.cache)
    }

    /// Returns `(rating_key, url)` for the first uncached track within
    /// `lookahead_depth` of the current queue position — a direct-play URL
    /// or a single-file transcode-download URL depending on the current
    /// `should_transcode` policy. Walks forward past already-cached
    /// entries. Returns `None` when every slot in the window is cached or
    /// out of bounds.
    ///
    /// Called fresh on every iteration of the prefetch worker's serial
    /// loop, so it auto-reflects queue advancement.
    pub fn next_uncached_target_in_lookahead(&self, include_current: bool) -> Option<(String, String)> {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let server_url = inner.server_url.as_ref()?;
        let token = inner.token.as_ref()?;

        let start_offset = if include_current { 0 } else { 1 };
        for offset in start_offset..=depth {
            let idx = pos + offset;
            let track = inner.state.queue.get(idx)?;

            if persistent.contains_key(&track.rating_key)
                || inner.cache.get(&track.rating_key).is_some()
            {
                continue;
            }

            // Prefetch inherits any adaptive step: a link that can't sustain
            // lossless live can't sustain it as a download either, and the
            // smaller file is what lets the lookahead actually get ahead —
            // which is the real fix, since a cached track plays perfectly.
            let (needs_transcode, bitrate) = effective_stream_policy(track, &inner);

            if needs_transcode {
                // Session shape is `<client-id>-<unique-id>` — Plex
                // tokenises on `-` for session grouping, so any extra
                // suffix risks the server conflating concurrent sessions
                // for the same client and quietly dropping one. Live and
                // prefetch for the same track use the same session value
                // (rating-key dedupes), but they only ever overlap if
                // the user is actively playing a track they're also
                // about to prefetch — and `next_uncached_target_in_lookahead`
                // already skips anything cached, so that's a no-op.
                let session = format!("{}-{}", inner.client_identifier, track.rating_key);
                let Some(url) = transcode::build_transcode_download_url(
                    server_url,
                    token,
                    &track.rating_key,
                    &inner.client_identifier,
                    &session,
                    bitrate,
                    None,
                ) else {
                    continue;
                };
                log::debug!(
                    "next_uncached_target: idx={idx} rk={} (transcoded prefetch)",
                    track.rating_key,
                );
                return Some((track.rating_key.clone(), url.to_string()));
            }

            let Some(part_key) = track.part_key.as_ref() else {
                continue;
            };
            let Some(url) = transcode::build_direct_play_url(server_url, part_key, token) else {
                continue;
            };
            // The constructed URL contains a live `X-Plex-Token` query
            // parameter, so only log the rating key + part key — never
            // the URL itself.
            log::debug!(
                "next_uncached_target: idx={idx} rk={} part_key={}",
                track.rating_key,
                part_key,
            );
            return Some((track.rating_key.clone(), url.to_string()));
        }
        None
    }

    /// Rating key of the currently playing track (for cache eviction protection).
    pub fn current_track_id(&self) -> Option<String> {
        self.inner
            .lock()
            .state
            .current_track
            .as_ref()
            .map(|t| t.rating_key.clone())
    }

    /// Returns `(rating_key, local_path)` for every track in the current
    /// playback queue's lookahead window that is already available on
    /// disk — either in the LRU prefetch cache or as a permanent download.
    /// Used by the download worker to drive spectrum analysis for
    /// already-cached tracks, which no longer trigger the prefetch
    /// success path that historically queued analysis.
    pub fn cached_paths_in_lookahead(
        &self,
        include_current: bool,
    ) -> Vec<(String, PathBuf)> {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let start_offset = if include_current { 0 } else { 1 };
        let mut out = Vec::new();
        for offset in start_offset..=depth {
            let idx = pos + offset;
            let Some(track) = inner.state.queue.get(idx) else {
                break;
            };
            if let Some(path) = persistent.get(&track.rating_key) {
                out.push((track.rating_key.clone(), path.clone()));
                continue;
            }
            if let Some(path) = inner.cache.get(&track.rating_key) {
                out.push((track.rating_key.clone(), path.to_path_buf()));
            }
        }
        out
    }

    /// Returns a `WarmTarget` for every lookahead-window track whose audio
    /// is already cached (LRU prefetch or permanent download), carrying its
    /// on-disk audio path and album-art thumb. Drives the prefetch worker's
    /// lowest-priority warming tier, which fills waveform sidecars and hero
    /// art only for tracks that are already playable offline.
    ///
    /// Re-read fresh on each warming pass, so it tracks queue advancement
    /// the same way `next_uncached_target_in_lookahead` does.
    pub fn lookahead_warm_targets(&self, include_current: bool) -> Vec<WarmTarget> {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let start_offset = if include_current { 0 } else { 1 };
        let mut out = Vec::new();
        for offset in start_offset..=depth {
            let idx = pos + offset;
            let Some(track) = inner.state.queue.get(idx) else {
                break;
            };
            // Audio-secured gate: a persistent download wins over the LRU
            // copy. Tracks with no cached audio are skipped entirely.
            let audio_path = if let Some(path) = persistent.get(&track.rating_key) {
                path.clone()
            } else if let Some(path) = inner.cache.get(&track.rating_key) {
                path.to_path_buf()
            } else {
                continue;
            };
            out.push(WarmTarget {
                rating_key: track.rating_key.clone(),
                audio_path,
                thumb: track.thumb.clone(),
            });
        }
        out
    }

    /// Swap a cached track's mpv playlist entry to `file://<path>` so mpv
    /// reads from the local cache file instead of re-downloading.
    ///
    /// Called by the prefetch worker after every successful download.
    /// No-op if the track isn't in the current queue or is the currently
    /// playing entry (mpv refuses to playlist-remove the active index).
    pub fn swap_playlist_entry_to_cached(&self, track_id: &str) {
        let (idx, file_url) = {
            let persistent = self.persistent_cache.read();
            let inner = self.inner.lock();
            let Some(idx) = inner
                .state
                .queue
                .iter()
                .position(|t| t.rating_key == track_id)
            else {
                return;
            };
            if idx == inner.state.queue_index {
                return;
            }
            let path = persistent
                .get(track_id)
                .cloned()
                .or_else(|| inner.cache.get(track_id).map(|p| p.to_path_buf()));
            let Some(path) = path else {
                return;
            };
            (idx, format!("file://{}", path.display()))
        };
        self.mpv.playlist_remove(idx as i64);
        self.mpv.load_file_at(&file_url, idx as i64, None);
    }
}
