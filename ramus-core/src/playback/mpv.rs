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
    /// Realtime audio metering via the `astats` filter. Observed as
    /// `af-metadata/astats`, returning a map of lavfi metadata keys.
    AudioLevel = 11,
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

/// Realtime audio metering snapshot from mpv's `astats` filter.
/// All values are in dBFS; `f64::NEG_INFINITY` means the channel is silent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioLevels {
    pub left_peak: f64,
    pub right_peak: f64,
    pub left_rms: f64,
    pub right_rms: f64,
}

/// Incremental builder for `AudioLevels`. The FFI event loop walks an
/// `mpv_node_list` of key/value string pairs and calls `apply_astats_entry`
/// for each one; when all four required keys have been seen, `build`
/// returns the finished snapshot. Having this as a pure safe type lets us
/// unit-test the parsing semantics without constructing raw mpv nodes.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct AudioLevelsBuilder {
    pub left_peak: Option<f64>,
    pub right_peak: Option<f64>,
    pub left_rms: Option<f64>,
    pub right_rms: Option<f64>,
}

impl AudioLevelsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a single (key, value-as-string) pair from `af-metadata/astats`.
    /// Unknown keys are ignored. Unparseable values are ignored. The string
    /// `"-inf"` is recognised as silence and mapped to `f64::NEG_INFINITY`.
    /// Returns whether the entry was consumed (true) or ignored (false).
    pub fn apply_astats_entry(&mut self, key: &str, value: &str) -> bool {
        let parsed = if value == "-inf" {
            f64::NEG_INFINITY
        } else {
            match value.parse::<f64>() {
                Ok(v) => v,
                Err(_) => return false,
            }
        };
        match key {
            "lavfi.astats.1.Peak_level" => {
                self.left_peak = Some(parsed);
                true
            }
            "lavfi.astats.1.RMS_level" => {
                self.left_rms = Some(parsed);
                true
            }
            "lavfi.astats.2.Peak_level" => {
                self.right_peak = Some(parsed);
                true
            }
            "lavfi.astats.2.RMS_level" => {
                self.right_rms = Some(parsed);
                true
            }
            _ => false,
        }
    }

    /// Finalise — returns `Some(AudioLevels)` only if all four required
    /// keys were seen. Returns `None` if any channel's peak or RMS is
    /// missing, so partial snapshots never propagate to the frontend.
    pub fn build(self) -> Option<AudioLevels> {
        Some(AudioLevels {
            left_peak: self.left_peak?,
            right_peak: self.right_peak?,
            left_rms: self.left_rms?,
            right_rms: self.right_rms?,
        })
    }
}

/// Callback signature for audio level updates.
pub type AudioLevelCallback = Box<dyn Fn(AudioLevels) + Send + Sync>;

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
    pub on_idle_active: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_loaded: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_ended: Option<Box<dyn Fn(FileEndReason) + Send + Sync>>,
    /// Realtime audio levels from the `astats` filter, in dB.
    pub on_audio_level: Option<AudioLevelCallback>,
}

// --- Configuration ---

/// Default mpv initialization options for audio-only playback.
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
        ("keep-open", "no"),
        ("idle", "yes"),
        ("input-default-bindings", "no"),
        ("input-vo-keyboard", "no"),
        ("terminal", "no"),
        ("load-scripts", "no"),
        ("msg-level", "all=warn"),
        // Base audio filter chain — always includes the labelled astats
        // metering filter so `af-metadata/astats` is observable from the
        // very first track. EQ is appended to this chain at runtime via
        // apply_equalizer() without dropping the astats segment.
        ("af", "@astats:astats=metadata=1:reset=1:measure_overall=none"),
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
        ("af-metadata/astats", ObserverID::AudioLevel),
    ]
}

// --- MpvHandle (trait for testability) ---

/// Abstraction over mpv operations for testability.
/// The real implementation wraps libmpv FFI calls.
pub trait MpvPlayer: Send + Sync {
    fn load_file(&self, url: &str, mode: LoadMode);
    fn load_file_at(&self, url: &str, index: i64);
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
        // astats metering must be seeded at init so the focus-mode
        // visualiser has data before the first EQ apply.
        assert!(opts
            .iter()
            .any(|(k, v)| *k == "af" && v.contains("astats=metadata=1")));
    }

    // --- AudioLevelsBuilder tests ---
    //
    // These mirror what `parse_astats_node` in ramus-tauri does after the
    // unsafe FFI walk: it feeds every key/value pair it sees into this
    // builder. Testing the pure builder lets us cover the parsing
    // semantics without constructing raw mpv nodes.

    #[test]
    fn test_audio_levels_builder_accepts_all_four_keys() {
        let mut b = AudioLevelsBuilder::new();
        assert!(b.apply_astats_entry("lavfi.astats.1.Peak_level", "-3.2"));
        assert!(b.apply_astats_entry("lavfi.astats.1.RMS_level", "-12.5"));
        assert!(b.apply_astats_entry("lavfi.astats.2.Peak_level", "-2.8"));
        assert!(b.apply_astats_entry("lavfi.astats.2.RMS_level", "-11.9"));
        let levels = b.build().expect("all four keys should produce a snapshot");
        assert!((levels.left_peak - -3.2).abs() < 1e-9);
        assert!((levels.right_peak - -2.8).abs() < 1e-9);
        assert!((levels.left_rms - -12.5).abs() < 1e-9);
        assert!((levels.right_rms - -11.9).abs() < 1e-9);
    }

    #[test]
    fn test_audio_levels_builder_handles_minus_inf() {
        let mut b = AudioLevelsBuilder::new();
        b.apply_astats_entry("lavfi.astats.1.Peak_level", "-inf");
        b.apply_astats_entry("lavfi.astats.1.RMS_level", "-inf");
        b.apply_astats_entry("lavfi.astats.2.Peak_level", "-inf");
        b.apply_astats_entry("lavfi.astats.2.RMS_level", "-inf");
        let levels = b.build().expect("silent snapshot should still build");
        assert_eq!(levels.left_peak, f64::NEG_INFINITY);
        assert_eq!(levels.right_peak, f64::NEG_INFINITY);
        assert_eq!(levels.left_rms, f64::NEG_INFINITY);
        assert_eq!(levels.right_rms, f64::NEG_INFINITY);
    }

    #[test]
    fn test_audio_levels_builder_missing_key_returns_none() {
        let mut b = AudioLevelsBuilder::new();
        b.apply_astats_entry("lavfi.astats.1.Peak_level", "-3.2");
        b.apply_astats_entry("lavfi.astats.1.RMS_level", "-12.5");
        b.apply_astats_entry("lavfi.astats.2.Peak_level", "-2.8");
        // Missing right RMS — should refuse to build a partial snapshot.
        assert!(b.build().is_none());
    }

    #[test]
    fn test_audio_levels_builder_ignores_unknown_keys() {
        let mut b = AudioLevelsBuilder::new();
        assert!(!b.apply_astats_entry("lavfi.astats.Overall.Peak_level", "-3.0"));
        assert!(!b.apply_astats_entry("lavfi.r128.M", "-18.0"));
        assert!(!b.apply_astats_entry("completely.unrelated", "42"));
        assert!(b.build().is_none());
    }

    #[test]
    fn test_audio_levels_builder_rejects_unparseable_values() {
        let mut b = AudioLevelsBuilder::new();
        assert!(!b.apply_astats_entry("lavfi.astats.1.Peak_level", "not a number"));
        assert!(!b.apply_astats_entry("lavfi.astats.1.RMS_level", ""));
        // Neither entry landed, so the builder is still empty.
        assert!(b.build().is_none());
    }

    #[test]
    fn test_audio_levels_builder_later_entry_overrides_earlier() {
        // Defence against a filter that emits the same key twice — last
        // write wins, which matches the behaviour of a naive hashmap
        // iteration.
        let mut b = AudioLevelsBuilder::new();
        b.apply_astats_entry("lavfi.astats.1.Peak_level", "-5.0");
        b.apply_astats_entry("lavfi.astats.1.Peak_level", "-2.0");
        b.apply_astats_entry("lavfi.astats.1.RMS_level", "-10.0");
        b.apply_astats_entry("lavfi.astats.2.Peak_level", "-3.0");
        b.apply_astats_entry("lavfi.astats.2.RMS_level", "-8.0");
        let levels = b.build().unwrap();
        assert!((levels.left_peak - -2.0).abs() < 1e-9);
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
            .any(|(name, id)| *name == "af-metadata/astats" && *id == ObserverID::AudioLevel));
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
        assert_eq!(ObserverID::AudioLevel as u64, 11);
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
        assert!(cb.on_idle_active.is_none());
        assert!(cb.on_file_loaded.is_none());
        assert!(cb.on_file_ended.is_none());
    }
}
