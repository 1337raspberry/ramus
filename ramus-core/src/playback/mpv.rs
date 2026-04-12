//! Thin wrapper around libmpv's C API for audio-only playback.
//!
//! The actual FFI calls require libmpv to be installed at runtime.
//! This module defines the wrapper types, configuration, and event model.
//! The real mpv handle is behind a cfg gate — tests and builds without
//! libmpv use the types and logic without linking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// --- Types ---

/// Reason a file ended playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEndReason {
    Eof,
    Stop,
    Quit,
    Error(String),
    Redirect,
    Unknown,
}

/// Property observer IDs — match Swift's ObserverID enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObserverID {
    TimePos = 1,
    Duration = 2,
    Pause = 3,
    PlaylistPos = 5,
    PausedForCache = 7,
    IdleActive = 9,
    CacheBufferingState = 10,
    /// mpv `cache-speed` (bytes/sec being read from the network by the
    /// demuxer). The prefetch worker watches this to detect when mpv has
    /// finished pulling the current track into its cache — that's the
    /// "safe to open a second HTTP connection for prefetch" signal.
    CacheSpeed = 11,
}

/// mpv file load mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    Replace,
    Append,
    AppendPlay,
}

impl LoadMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Append => "append",
            Self::AppendPlay => "append-play",
        }
    }
}

// --- Callbacks ---

/// Callbacks fired by the mpv event loop.
/// All callbacks are `Send + Sync` so they can be dispatched from the event thread.
#[derive(Default)]
pub struct MpvCallbacks {
    pub on_position_change: Option<Box<dyn Fn(f64) + Send + Sync>>,
    pub on_duration_change: Option<Box<dyn Fn(f64) + Send + Sync>>,
    pub on_playlist_pos_change: Option<Box<dyn Fn(i64) + Send + Sync>>,
    pub on_pause_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    pub on_buffering_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    pub on_cache_state_change: Option<Box<dyn Fn(i64) + Send + Sync>>,
    pub on_cache_speed_change: Option<Box<dyn Fn(f64) + Send + Sync>>,
    pub on_idle_active: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_loaded: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_ended: Option<Box<dyn Fn(FileEndReason) + Send + Sync>>,
}

// --- Configuration ---

/// Default mpv initialization options for audio-only playback.
///
/// Note: the `af` chain is deliberately empty by default. The old
/// `astats` metering filter was removed when the focus visualiser was
/// rewritten to use precomputed per-track spectrograms — that pipeline
/// doesn't need any live filter metadata. Runtime EQ changes call
/// `set_audio_filters()` with the EQ chain directly (or an empty
/// string to clear it); nothing else touches the `af` property.
pub fn default_mpv_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vo", "null"),
        ("vid", "no"),
        #[cfg(target_os = "macos")]
        ("ao", "coreaudio"),
        #[cfg(target_os = "windows")]
        ("ao", "wasapi"),
        #[cfg(target_os = "linux")]
        ("ao", "pipewire"),
        ("gapless-audio", "yes"),
        ("prefetch-playlist", "yes"),
        ("audio-buffer", "0.5"),
        // Let mpv eagerly pull the whole file into its demuxer cache, so
        // `cache-speed` reliably drops to 0 within ~1 minute of playback
        // start. The prefetch worker watches for that idle signal before
        // it opens a second HTTP connection to Plex. Without these, a
        // large FLAC would trickle all the way through playback and the
        // worker would never get its "safe to start prefetching" go-ahead.
        ("demuxer-max-bytes", "2GiB"),
        ("demuxer-readahead-secs", "1200"),
        ("keep-open", "no"),
        ("idle", "yes"),
        ("input-default-bindings", "no"),
        ("input-vo-keyboard", "no"),
        ("terminal", "no"),
        ("load-scripts", "no"),
        ("msg-level", "all=warn"),
    ]
}

/// Properties to observe after mpv initialization.
pub fn observed_properties() -> Vec<(&'static str, ObserverID)> {
    vec![
        ("time-pos", ObserverID::TimePos),
        ("duration", ObserverID::Duration),
        ("pause", ObserverID::Pause),
        ("playlist-pos", ObserverID::PlaylistPos),
        ("paused-for-cache", ObserverID::PausedForCache),
        ("idle-active", ObserverID::IdleActive),
        ("cache-buffering-state", ObserverID::CacheBufferingState),
        ("cache-speed", ObserverID::CacheSpeed),
    ]
}

// --- MpvHandle (trait for testability) ---

/// Abstraction over mpv operations for testability.
/// The real implementation wraps libmpv FFI calls.
pub trait MpvPlayer: Send + Sync {
    /// Load a file into the playlist. `options` is an mpv-style
    /// comma-separated `key=value` list applied to just this entry — used
    /// by the prefetch layer to set `stream-record=<path>` so mpv writes
    /// the received bytes to disk as it plays, giving us a cache file
    /// without a second HTTP connection.
    fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>);
    fn load_file_at(&self, url: &str, index: i64, options: Option<&str>);
    fn playlist_play_index(&self, index: i64);
    fn playlist_remove(&self, index: i64);
    fn playlist_move(&self, from: i64, to: i64);
    fn seek(&self, position: f64);
    fn set_pause(&self, paused: bool);
    fn set_volume(&self, volume: f64);
    fn get_volume(&self) -> f64;
    fn set_audio_filters(&self, value: &str);
    fn stop(&self);
    fn is_shutdown(&self) -> bool;
}

// --- Shutdown flag (shared between controller and event loop) ---

/// Thread-safe shutdown flag. Prevents use-after-free when the mpv handle
/// is destroyed while event loop callbacks are still in-flight.
#[derive(Clone)]
pub struct ShutdownFlag {
    flag: Arc<AtomicBool>,
}

impl ShutdownFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set(&self) {
        self.flag.store(true, Ordering::Release);
    }

    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

impl Default for ShutdownFlag {
    fn default() -> Self {
        Self::new()
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_mode_strings() {
        assert_eq!(LoadMode::Replace.as_str(), "replace");
        assert_eq!(LoadMode::Append.as_str(), "append");
        assert_eq!(LoadMode::AppendPlay.as_str(), "append-play");
    }

    #[test]
    fn test_shutdown_flag() {
        let flag = ShutdownFlag::new();
        assert!(!flag.is_set());
        flag.set();
        assert!(flag.is_set());
    }

    #[test]
    fn test_shutdown_flag_clone_shares_state() {
        let flag1 = ShutdownFlag::new();
        let flag2 = flag1.clone();
        assert!(!flag2.is_set());
        flag1.set();
        assert!(flag2.is_set());
    }

    #[test]
    fn test_default_mpv_options_not_empty() {
        let opts = default_mpv_options();
        assert!(!opts.is_empty());
        // Must include audio-only options
        assert!(opts.iter().any(|(k, v)| *k == "vo" && *v == "null"));
        assert!(opts.iter().any(|(k, v)| *k == "vid" && *v == "no"));
        assert!(opts.iter().any(|(k, v)| *k == "gapless-audio" && *v == "yes"));
        assert!(opts.iter().any(|(k, v)| *k == "idle" && *v == "yes"));
        assert!(opts.iter().any(|(k, v)| *k == "keep-open" && *v == "no"));
        // The focus visualiser now uses precomputed spectrograms, so no
        // `af` option should be seeded at init — the chain must start empty.
        assert!(!opts.iter().any(|(k, _)| *k == "af"));
    }

    #[test]
    fn test_observed_properties_complete() {
        let props = observed_properties();
        assert_eq!(props.len(), 8);
        assert!(props.iter().any(|(name, id)| *name == "time-pos" && *id == ObserverID::TimePos));
        assert!(props.iter().any(|(name, id)| *name == "duration" && *id == ObserverID::Duration));
        assert!(props.iter().any(|(name, id)| *name == "pause" && *id == ObserverID::Pause));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "playlist-pos" && *id == ObserverID::PlaylistPos));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "paused-for-cache" && *id == ObserverID::PausedForCache));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "idle-active" && *id == ObserverID::IdleActive));
        assert!(props.iter().any(|(name, id)| *name == "cache-buffering-state"
            && *id == ObserverID::CacheBufferingState));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "cache-speed" && *id == ObserverID::CacheSpeed));
    }

    #[test]
    fn test_observer_id_values() {
        assert_eq!(ObserverID::TimePos as u64, 1);
        assert_eq!(ObserverID::Duration as u64, 2);
        assert_eq!(ObserverID::Pause as u64, 3);
        assert_eq!(ObserverID::PlaylistPos as u64, 5);
        assert_eq!(ObserverID::PausedForCache as u64, 7);
        assert_eq!(ObserverID::IdleActive as u64, 9);
        assert_eq!(ObserverID::CacheBufferingState as u64, 10);
        assert_eq!(ObserverID::CacheSpeed as u64, 11);
    }

    #[test]
    fn test_file_end_reason_variants() {
        let reasons = [
            FileEndReason::Eof,
            FileEndReason::Stop,
            FileEndReason::Quit,
            FileEndReason::Error("test error".into()),
            FileEndReason::Redirect,
            FileEndReason::Unknown,
        ];
        assert_eq!(reasons.len(), 6);
        assert_eq!(
            FileEndReason::Error("test".into()),
            FileEndReason::Error("test".into())
        );
        assert_ne!(FileEndReason::Eof, FileEndReason::Stop);
    }

    #[test]
    fn test_callbacks_default() {
        let cb = MpvCallbacks::default();
        assert!(cb.on_position_change.is_none());
        assert!(cb.on_duration_change.is_none());
        assert!(cb.on_playlist_pos_change.is_none());
        assert!(cb.on_pause_change.is_none());
        assert!(cb.on_buffering_change.is_none());
        assert!(cb.on_cache_state_change.is_none());
        assert!(cb.on_cache_speed_change.is_none());
        assert!(cb.on_idle_active.is_none());
        assert!(cb.on_file_loaded.is_none());
        assert!(cb.on_file_ended.is_none());
    }
}
