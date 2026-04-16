//! No-op `MpvPlayer` implementation used on iOS during Phase 1 of the iOS
//! port. Keeps the rest of the crate compiling unchanged — `AudioPlayer`,
//! `SessionReporter`, prefetch and the command handlers all wrap or call
//! into this trait, so providing a benign stub avoids cascading `#[cfg]`
//! gates across unrelated modules.
//!
//! Replaced in Phase 2 by the MPVKit-backed implementation bridged through
//! a Swift plugin. Until then the simulator will launch with no audio: the
//! UI renders, the library browses, and playback commands silently succeed.

use std::sync::Arc;

use ramus_core::playback::mpv::{LoadMode, MpvPlayer};
use ramus_core::playback::player::AudioPlayer;

pub struct StubMpvPlayer;

impl StubMpvPlayer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubMpvPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MpvPlayer for StubMpvPlayer {
    fn load_file(&self, _url: &str, _mode: LoadMode, _options: Option<&str>) {}
    fn load_file_at(&self, _url: &str, _index: i64, _options: Option<&str>) {}
    fn playlist_play_index(&self, _index: i64) {}
    fn playlist_remove(&self, _index: i64) {}
    fn playlist_move(&self, _from: i64, _to: i64) {}
    fn seek(&self, _position: f64) {}
    fn set_pause(&self, _paused: bool) {}
    fn set_volume(&self, _volume: f64) {}
    fn get_volume(&self) -> f64 {
        100.0
    }
    fn set_audio_filters(&self, _value: &str) {}
    fn stop(&self) {}
    fn is_shutdown(&self) -> bool {
        false
    }
}

pub fn create_stub_audio_player() -> Arc<AudioPlayer> {
    Arc::new(AudioPlayer::new(Arc::new(StubMpvPlayer::new())))
}
