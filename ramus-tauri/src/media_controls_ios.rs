//! iOS media controls. Subscribes to the Swift plugin's remote-command
//! events (lock-screen + Control Center buttons) and dispatches them to
//! the shared `AudioPlayer`. Implements the same `MediaKeyHandler`
//! surface as the desktop souvlaki wrapper so the rest of the crate
//! calls through unchanged; on iOS `update_metadata` pushes to
//! `MPNowPlayingInfoCenter` via the plugin instead of souvlaki.

use std::sync::Arc;

use tauri::{ipc::Channel, AppHandle};
use tauri_plugin_ramus_ios_bridge::{NowPlayingMetadata, RamusIosBridgeExt};

use ramus_core::cache::image_cache::ImageCache;
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::player::AudioPlayer;
use ramus_core::plex::client::PlexClient;

pub type MediaControlsRef = Arc<parking_lot::Mutex<Option<MediaControlsHandle>>>;

/// Frontend art cache sizes, from `ui/src/lib/commands.ts::ART_SIZE`,
/// in priority order: grid (300) → now-playing large (1200) → compact (72).
const ART_SIZES_TO_TRY: &[u32] = &[300, 1200, 72];

pub struct MediaControlsHandle {
    app: AppHandle,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    /// Cached last full metadata, used by `update_playback_state` to
    /// re-push the lock-screen widget with an updated rate/position.
    /// `MPNowPlayingInfoCenter` wants a full dict each update — sending
    /// only transport state would blank out title/artist/artwork on the
    /// lock screen.
    last_metadata: parking_lot::Mutex<Option<NowPlayingMetadata>>,
}

impl MediaControlsHandle {
    /// Resolve album art from the image cache, returning a `file://` URL
    /// for the Swift plugin to load. Only hits the cache — no network —
    /// so a missed lookup returns `None` and the NowPlaying widget uses
    /// whatever artwork it already has.
    fn resolve_cover_art(&self, thumb: Option<&str>) -> Option<String> {
        let thumb = thumb?;
        let mut cache = self.image_cache.lock();
        ART_SIZES_TO_TRY
            .iter()
            .find_map(|&size| cache.get(thumb, size))
            .map(|p| format!("file://{}", p.display()))
    }
}

impl MediaKeyHandler for MediaControlsHandle {
    fn update_metadata(&self, metadata: &MediaMetadata) {
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
        if let Err(e) = self.app.ramus_ios_bridge().now_playing_update(payload) {
            log::warn!("nowPlayingUpdate failed: {e}");
        }
    }

    fn update_playback_state(&self, is_playing: bool, position: f64) {
        let mut guard = self.last_metadata.lock();
        let Some(ref mut meta) = *guard else { return };
        meta.is_playing = is_playing;
        meta.position = position.max(0.0);
        let payload = meta.clone();
        drop(guard);
        if let Err(e) = self.app.ramus_ios_bridge().now_playing_update(payload) {
            log::warn!("nowPlayingUpdate (state) failed: {e}");
        }
    }

    fn clear(&self) {
        *self.last_metadata.lock() = None;
        if let Err(e) = self.app.ramus_ios_bridge().now_playing_clear() {
            log::warn!("nowPlayingClear failed: {e}");
        }
    }
}

/// Build the iOS media-controls handle and wire the six lock-screen /
/// Control Center events into `AudioPlayer`. Every method on
/// `AudioPlayer` is thread-safe (internal `parking_lot::Mutex`) so the
/// listeners can dispatch directly.
pub fn create_media_controls(
    app: AppHandle,
    player: Arc<AudioPlayer>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    _client: Arc<PlexClient>,
    _http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    // Each remote-command event goes through its own `Channel` so the
    // Swift side's `trigger(_:data:)` can reach us — plugin events are
    // not deliverable via `app.listen()`.
    let bridge = app.ramus_ios_bridge();

    for (name, action) in [
        (
            "remotePlay",
            Box::new({
                let p = player.clone();
                move || p.resume()
            }) as Box<dyn Fn() + Send + Sync>,
        ),
        (
            "remotePause",
            Box::new({
                let p = player.clone();
                move || p.pause()
            }),
        ),
        (
            "remoteToggle",
            Box::new({
                let p = player.clone();
                move || p.toggle_play_pause()
            }),
        ),
        (
            "remoteNext",
            Box::new({
                let p = player.clone();
                move || p.next()
            }),
        ),
        (
            "remotePrevious",
            Box::new({
                let p = player.clone();
                move || p.previous()
            }),
        ),
    ] {
        let channel = Channel::new(move |_body| {
            action();
            Ok(())
        });
        bridge
            .register_listener(name, channel)
            .map_err(|e| format!("register {name}: {e}"))?;
    }

    // seek carries a `position: f64` in its payload.
    {
        let p = player.clone();
        let channel = Channel::new(move |body| {
            if let Ok(payload) = body.deserialize::<serde_json::Value>() {
                if let Some(pos) = payload.get("position").and_then(|v| v.as_f64()) {
                    p.seek(pos);
                }
            }
            Ok(())
        });
        bridge
            .register_listener("remoteSeek", channel)
            .map_err(|e| format!("register remoteSeek: {e}"))?;
    }

    Ok(MediaControlsHandle {
        app,
        image_cache,
        last_metadata: parking_lot::Mutex::new(None),
    })
}
