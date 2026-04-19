//! Android stub `MpvPlayer`. No audio backend is wired yet — every method
//! is a no-op so the rest of `ramus-tauri` (AudioPlayer, SessionReporter,
//! prefetch, commands) compiles and runs unchanged. Real playback will
//! come from either bundled mpv-android `.so`s or an ExoPlayer/Media3
//! Kotlin bridge.
//!
//! Callbacks registered by `AudioPlayer` are held so a future backend
//! can forward property changes into them; for now nothing fires.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use ramus_core::playback::mpv::{LoadMode, MpvCallbacks, MpvPlayer};

pub struct AndroidMpvPlayer {
    _callbacks: Arc<MpvCallbacks>,
    shutdown: AtomicBool,
    cached_volume: AtomicU64,
}

impl AndroidMpvPlayer {
    pub fn new(callbacks: Arc<MpvCallbacks>) -> Self {
        Self {
            _callbacks: callbacks,
            shutdown: AtomicBool::new(false),
            cached_volume: AtomicU64::new(100.0_f64.to_bits()),
        }
    }
}

impl MpvPlayer for AndroidMpvPlayer {
    fn load_file(&self, _url: &str, _mode: LoadMode, _options: Option<&str>) {}
    fn load_file_at(&self, _url: &str, _index: i64, _options: Option<&str>) {}
    fn playlist_play_index(&self, _index: i64) {}
    fn playlist_remove(&self, _index: i64) {}
    fn playlist_move(&self, _from: i64, _to: i64) {}
    fn seek(&self, _position: f64) {}
    fn set_pause(&self, _paused: bool) {}
    fn set_volume(&self, volume: f64) {
        self.cached_volume.store(volume.to_bits(), Ordering::Release);
    }
    fn get_volume(&self) -> f64 {
        f64::from_bits(self.cached_volume.load(Ordering::Acquire))
    }
    fn set_audio_filters(&self, _value: &str) {}
    fn stop(&self) {}
    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}
