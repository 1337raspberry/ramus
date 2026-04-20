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

/// Frontend art cache sizes, from `ui/src/lib/commands.ts::ART_SIZE`,
/// in priority order: grid (300) → now-playing large (1200) → compact (72).
const ART_SIZES_TO_TRY: &[u32] = &[300, 1200, 72];

pub struct MediaControlsHandle {
    app: AppHandle,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    /// Cached last full metadata. `update_playback_state` re-pushes the
    /// session with the new transport state but unchanged metadata; the
    /// Kotlin side checks the cover URL hasn't changed before reloading
    /// artwork bytes.
    last_metadata: parking_lot::Mutex<Option<NowPlayingMetadata>>,
}

impl MediaControlsHandle {
    fn resolve_cover_art(&self, thumb: Option<&str>) -> Option<String> {
        let thumb = thumb?;
        let mut cache = self.image_cache.lock();
        ART_SIZES_TO_TRY
            .iter()
            .find_map(|&size| cache.get(thumb, size))
            .map(|p| format!("file://{}", p.display()))
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
        let cover_url = self.resolve_cover_art(metadata.cover_url.as_deref());
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
        self.dispatch_now_playing_clear();
    }
}

pub fn create_media_controls(
    app: AppHandle,
    _player: Arc<AudioPlayer>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    _client: Arc<PlexClient>,
    _http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    Ok(MediaControlsHandle {
        app,
        image_cache,
        last_metadata: parking_lot::Mutex::new(None),
    })
}
