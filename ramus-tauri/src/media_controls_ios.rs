//! iOS stub for `media_controls`. Mirrors the public surface of the
//! souvlaki-backed desktop module so `AppState` and command handlers
//! compile unchanged. Every method is a no-op; `create_media_controls`
//! always succeeds and returns a handle that does nothing when invoked.
//!
//! Phase 2 replaces this with a Swift-plugin bridge to
//! `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`.

use std::sync::Arc;

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
    _player: Arc<AudioPlayer>,
    _image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    _client: Arc<PlexClient>,
    _http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    Ok(MediaControlsHandle)
}
