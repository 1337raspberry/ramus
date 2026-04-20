//! Android media controls. Forwards `update_metadata` /
//! `update_playback_state` / `clear` to the Kotlin `MpvBridgePlugin`,
//! which keeps a Media3 `MediaSession` in sync. The session + foreground
//! service drive the lock-screen, Bluetooth, and Auto controls; the
//! existing player listener feeds transport-state changes back to Rust
//! through `mpvPauseChange` events, so MediaSession callbacks
//! (lock-screen play/pause/seek) automatically route through the same
//! pipeline as in-app actions without a separate remote-command channel.
//!
//! Mirrors the public surface of `media_controls_ios.rs` so `lib.rs`
//! wires through unchanged.
//!
//! **All `bridge.*` IPC calls are dispatched via
//! `tauri::async_runtime::spawn_blocking`**. Calling `run_mobile_plugin`
//! synchronously from the Tauri event-callback thread deadlocks with the
//! Android main looper — the JS layer dispatches the trigger event on
//! main and waits for the Rust callback to return; the Rust callback
//! waits for `run_mobile_plugin` which waits for the Kotlin `@Command`
//! which queues on main, behind the JS event we're still inside.

use std::process::Command;
use std::sync::{Arc, OnceLock};

use tauri::AppHandle;
use tauri_plugin_ramus_ios_bridge::{NowPlayingMetadata, RamusIosBridgeExt};

use ramus_core::cache::image_cache::ImageCache;
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::player::AudioPlayer;
use ramus_core::plex::client::PlexClient;
use ramus_core::util::plex_art_url;

pub type MediaControlsRef = Arc<parking_lot::Mutex<Option<MediaControlsHandle>>>;

/// Debug bypass: if `debug.ramus.media_controls` is set to `0`/`false`/`off`,
/// every method on `MediaControlsHandle` becomes a no-op so we can A/B
/// whether the bridge IPC chain is causing playback issues. Read once on
/// first call so we don't shell out on every position tick.
fn media_controls_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let raw = Command::new("getprop")
            .arg("debug.ramus.media_controls")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let disabled = matches!(raw.as_str(), "0" | "false" | "off");
        if disabled {
            log::warn!("media_controls disabled via debug.ramus.media_controls");
        }
        !disabled
    })
}

/// Priority order when resolving cached art. High-quality first so the
/// lock-screen widget gets a crisp image whenever the UI has already
/// pulled the 300/1200 variant. 72px is the last-resort fallback used
/// by tiny surfaces (search rows, queue thumbnails); it's technically
/// a hit but visibly blurry on the widget, so we treat it as "low-res"
/// and still kick off the 300 download in the background.
const ART_SIZES_TO_TRY: &[u32] = &[300, 1200, 72];

/// Size at which we self-download art when the cache miss is total or
/// only held the 72px thumb. Matches the desktop value so the same art
/// blob serves the album grid AND the lock-screen widget with one fetch.
const ART_DOWNLOAD_SIZE: u32 = 300;

/// Memoised art-resolution state. `thumb` is the stable Plex thumb key
/// (e.g. `/library/metadata/123/thumb/456789`); `cover_url` is the
/// `file://…` pointer we last handed the Kotlin bridge. A same-thumb
/// repeat check returns the cached URL without re-hitting the image
/// cache. A background download only overwrites this struct if the
/// thumb it was started for still matches — rapid skipping can't paint
/// the new track's widget with the old track's art.
#[derive(Default)]
struct CachedArt {
    thumb: Option<String>,
    cover_url: Option<String>,
    /// True once we've resolved this thumb from a 300+ cache entry.
    /// Gates the background download so a 72px-only thumb triggers one
    /// fetch, not one per metadata refresh.
    high_res: bool,
}

pub struct MediaControlsHandle {
    app: AppHandle,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
    /// Cached last full metadata. `update_playback_state` re-pushes the
    /// session with the new transport state but unchanged metadata; the
    /// Kotlin side checks the cover URL hasn't changed before reloading
    /// artwork bytes.
    last_metadata: parking_lot::Mutex<Option<NowPlayingMetadata>>,
    cached_art: Arc<parking_lot::Mutex<CachedArt>>,
}

impl MediaControlsHandle {
    /// Resolve album art from the image cache. Returns `(url, high_res)`
    /// — `high_res` is false when we only found the 72px thumb, which
    /// tells `update_metadata` to kick off a background 300 download.
    fn resolve_cover_art(&self, thumb: Option<&str>) -> (Option<String>, bool) {
        let Some(thumb) = thumb else {
            return (None, false);
        };

        // Fast path: same thumb as last time and we already committed
        // a high-quality URL. Avoids re-locking the image cache on
        // every position tick.
        {
            let cached = self.cached_art.lock();
            if cached.thumb.as_deref() == Some(thumb)
                && cached.high_res
                && cached.cover_url.is_some()
            {
                return (cached.cover_url.clone(), true);
            }
        }

        // Search the cache size-by-size; remember whether the hit was
        // at a high-quality size so the caller can decide to queue a
        // background fetch.
        let hit = {
            let mut cache = self.image_cache.lock();
            ART_SIZES_TO_TRY.iter().find_map(|&size| {
                cache
                    .get(thumb, size)
                    .map(|p| (p, size >= ART_DOWNLOAD_SIZE))
            })
        };

        let (cover_url, high_res) = match hit {
            Some((path, hi)) => (Some(format!("file://{}", path.display())), hi),
            None => (None, false),
        };

        // Only commit a high-quality result to the fast path. A 72-only
        // resolution leaves `high_res = false` so the next call retries
        // the cache (the background download may have landed since).
        {
            let mut cached = self.cached_art.lock();
            cached.thumb = Some(thumb.to_string());
            cached.cover_url = cover_url.clone();
            cached.high_res = high_res;
        }

        (cover_url, high_res)
    }
}

impl MediaControlsHandle {
    /// Spawn a blocking task to run the bridge call off the Tauri
    /// event-callback thread. See module docs for why this is mandatory.
    fn dispatch_now_playing(&self, payload: NowPlayingMetadata) {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = app.ramus_ios_bridge().now_playing_update(payload) {
                log::warn!("nowPlayingUpdate failed: {e}");
            }
        });
    }

    fn dispatch_now_playing_clear(&self) {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = app.ramus_ios_bridge().now_playing_clear() {
                log::warn!("nowPlayingClear failed: {e}");
            }
        });
    }
}

impl MediaKeyHandler for MediaControlsHandle {
    fn update_metadata(&self, metadata: &MediaMetadata) {
        if !media_controls_enabled() {
            return;
        }
        let (cover_url, high_res) = self.resolve_cover_art(metadata.cover_url.as_deref());
        let payload = NowPlayingMetadata {
            title: metadata.title.clone(),
            artist: metadata.artist.clone(),
            album: metadata.album.clone(),
            duration: metadata.duration,
            position: metadata.position.max(0.0),
            is_playing: metadata.is_playing,
            cover_url,
        };
        *self.last_metadata.lock() = Some(payload.clone());
        self.dispatch_now_playing(payload);

        // Either a total miss or a low-res-only hit (72px) — kick off a
        // 300px download in the background so the next metadata push
        // paints the widget at high quality. The download completion
        // self-dispatches a fresh `now_playing_update`, which goes
        // through Kotlin's coverChanged branch and reloads the bytes.
        if !high_res {
            if let Some(thumb) = metadata.cover_url.as_deref() {
                spawn_art_download(
                    thumb.to_string(),
                    metadata.clone(),
                    self.app.clone(),
                    self.image_cache.clone(),
                    self.client.clone(),
                    self.http_client.clone(),
                    self.cached_art.clone(),
                );
            }
        }
    }

    fn update_playback_state(&self, is_playing: bool, position: f64) {
        if !media_controls_enabled() {
            return;
        }
        let mut guard = self.last_metadata.lock();
        let Some(ref mut meta) = *guard else { return };
        meta.is_playing = is_playing;
        meta.position = position.max(0.0);
        let payload = meta.clone();
        drop(guard);
        self.dispatch_now_playing(payload);
    }

    fn clear(&self) {
        if !media_controls_enabled() {
            return;
        }
        *self.last_metadata.lock() = None;
        *self.cached_art.lock() = CachedArt::default();
        self.dispatch_now_playing_clear();
    }
}

/// Download album art from Plex in the background, insert into the
/// image cache, then re-dispatch `nowPlayingUpdate` so the lock-screen
/// widget swaps its low-res (or missing) artwork for the 300px variant.
///
/// Stale-guard: if the track changes mid-flight (detected via
/// `cached_art.thumb`), the result is discarded — rapid skipping
/// through a queue would otherwise paint the newer track's widget
/// with an older track's art.
fn spawn_art_download(
    thumb: String,
    metadata: MediaMetadata,
    app: AppHandle,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
    cached_art: Arc<parking_lot::Mutex<CachedArt>>,
) {
    tauri::async_runtime::spawn(async move {
        let server_url = match client.server_url() {
            Some(u) => u,
            None => return,
        };
        let token = match client.token() {
            Some(t) => t,
            None => return,
        };

        let url = plex_art_url(&server_url, &thumb, ART_DOWNLOAD_SIZE);

        let response = match http_client
            .get(&url)
            .header("X-Plex-Token", &token)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(_) => return,
        };

        let cover_url = {
            let mut cache = image_cache.lock();
            cache
                .insert(&thumb, ART_DOWNLOAD_SIZE, &bytes)
                .ok()
                .map(|p| format!("file://{}", p.display()))
        };

        // Guard against stale results: if another track has been
        // requested since we started, drop this one. Matching thumb is
        // the stable identifier across Plex rating-key changes.
        let Some(url) = cover_url else { return };
        {
            let mut art = cached_art.lock();
            if art.thumb.as_deref() != Some(&thumb) {
                return;
            }
            art.cover_url = Some(url.clone());
            art.high_res = true;
        }

        // Re-push via the bridge. Off the Tauri event-callback thread
        // because we're already on a tokio worker (spawn_blocking would
        // just wrap one worker in another); the bridge IPC is
        // non-reentrant in this context.
        let payload = NowPlayingMetadata {
            title: metadata.title.clone(),
            artist: metadata.artist.clone(),
            album: metadata.album.clone(),
            duration: metadata.duration,
            position: metadata.position.max(0.0),
            is_playing: metadata.is_playing,
            cover_url: Some(url),
        };
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = app.ramus_ios_bridge().now_playing_update(payload) {
                log::warn!("nowPlayingUpdate (art refresh) failed: {e}");
            }
        });
    });
}

pub fn create_media_controls(
    app: AppHandle,
    _player: Arc<AudioPlayer>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    Ok(MediaControlsHandle {
        app,
        image_cache,
        client,
        http_client,
        last_metadata: parking_lot::Mutex::new(None),
        cached_art: Arc::new(parking_lot::Mutex::new(CachedArt::default())),
    })
}
