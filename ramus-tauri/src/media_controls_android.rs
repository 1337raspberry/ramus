//! Android media-controls stub. Matches the public surface of
//! `media_controls_ios.rs` (`MediaControlsHandle`, `MediaControlsRef`,
//! `create_media_controls`) so `lib.rs` wires through unchanged, but every
//! method is a no-op. A proper integration would talk to
//! `MediaSessionCompat` / Media3 `MediaSession` via a Kotlin bridge so the
//! lock screen, Bluetooth remotes, and Auto pick up Now Playing.

use std::sync::Arc;

use tauri::AppHandle;

use ramus_core::cache::image_cache::ImageCache;
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::player::AudioPlayer;
use ramus_core::plex::client::PlexClient;

pub type MediaControlsRef = Arc<parking_lot::Mutex<Option<MediaControlsHandle>>>;

pub struct MediaControlsHandle;

impl MediaKeyHandler for MediaControlsHandle {
    fn update_metadata(&self, _metadata: &MediaMetadata) {}
    fn update_playback_state(&self, _is_playing: bool, _position: f64) {}
    fn clear(&self) {}
}

pub fn create_media_controls(
    _app: AppHandle,
    _player: Arc<AudioPlayer>,
    _image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    _client: Arc<PlexClient>,
    _http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    Ok(MediaControlsHandle)
}
