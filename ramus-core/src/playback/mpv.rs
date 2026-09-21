//! Wrapper types, configuration, and event model around libmpv's C API
//! for audio-only playback. FFI calls require libmpv at runtime; the real
//! mpv handle lives behind a cfg gate so tests and builds without libmpv
//! exercise the types and logic without linking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::playback::spectrum_tap::TapFrame;

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

/// Property observer IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObserverID {
    TimePos = 1,
    Duration = 2,
    Pause = 3,
    PlaylistPos = 5,
    IdleActive = 9,
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

/// Callbacks fired by the mpv event loop. All are `Send + Sync` so they
/// can be dispatched from the event thread.
#[derive(Default)]
pub struct MpvCallbacks {
    pub on_position_change: Option<Box<dyn Fn(f64) + Send + Sync>>,
    pub on_duration_change: Option<Box<dyn Fn(f64) + Send + Sync>>,
    pub on_playlist_pos_change: Option<Box<dyn Fn(i64) + Send + Sync>>,
    pub on_pause_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    pub on_idle_active: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_loaded: Option<Box<dyn Fn() + Send + Sync>>,
    pub on_file_ended: Option<Box<dyn Fn(FileEndReason) + Send + Sync>>,
    /// A batch of live spectrum tap frames parsed out of mpv's log stream.
    /// Frames arrive in bursts (mpv runs the filter chain when the audio
    /// output needs data), so the event loop hands over everything it
    /// collected in one drain rather than one frame at a time. Invoked on
    /// the mpv event-loop thread.
    pub on_spectrum_frames: Option<Box<dyn Fn(Vec<TapFrame>) + Send + Sync>>,
}

/// Default mpv initialization options for audio-only playback.
///
/// The `af` chain is deliberately empty by default: the equalizer and the
/// spectrum tap are composed at runtime through `set_audio_filters()` (see
/// `player/eq.rs`), so nothing needs seeding here.
pub fn default_mpv_options() -> Vec<(&'static str, &'static str)> {
    // mpv's own `prefetch-playlist` stays OFF. It would open the next
    // entry's Plex URL while the current one is still streaming, which is
    // exactly the second session the serial prefetch worker exists to
    // avoid: Plex cuts a live transcode when another one opens, and drops
    // concurrent remote downloads from the same client. The worker
    // downloads upcoming tracks itself and swaps mpv's playlist entries to
    // `file://` URLs as the bytes land, so gapless transitions still work
    // for anything cached before the previous track ends. The only cost is
    // a fast forward-skip into a not-yet-cached track, where mpv opens a
    // fresh HTTP source from scratch: a small audible gap, acceptable for
    // an interactive skip.
    vec![
        ("vo", "null"),
        ("vid", "no"),
        #[cfg(target_os = "macos")]
        ("ao", "coreaudio"),
        #[cfg(target_os = "windows")]
        ("ao", "wasapi"),
        // Linux: pulse → alsa → jack. We deliberately leave pipewire
        // OFF the list even though it's the dominant audio daemon on
        // modern distros, for two reasons:
        //
        // 1. The AppImage's bundled libmpv (Ubuntu 22.04 / mpv 0.34.1)
        //    has no pipewire AO compiled in — Debian/Ubuntu didn't
        //    enable it that long ago. Listing pipewire first guarantees
        //    a noisy "Audio output pipewire not found!" probe on every
        //    session with zero upside.
        // 2. On modern PipeWire-based systems (Ubuntu 24+, Fedora 38+,
        //    Arch, etc.), pipewire-pulse ships the PulseAudio API
        //    alongside the daemon. mpv's `pulse` AO transparently goes
        //    through that shim to the same daemon — for music playback
        //    the latency difference vs direct-pipewire is microseconds,
        //    imperceptible.
        //
        // mpv's comma-list semantics already do "try in order, stop at
        // first success, stay on it"; no runtime probe needed.
        #[cfg(target_os = "linux")]
        ("ao", "pulse,alsa,jack"),
        ("gapless-audio", "yes"),
        ("prefetch-playlist", "no"),
        ("audio-buffer", "0.5"),
        // Eagerly pull whole files into the demuxer cache for reliable
        // gapless playback of large files.
        ("demuxer-max-bytes", "2GiB"),
        ("demuxer-readahead-secs", "1200"),
        // Cap how long mpv will sit on a stalled HTTP request before erroring
        // out — without this, an unreachable host hangs forever and our
        // file-ended retry path never fires. Paired with lavf reconnect so
        // transient drops auto-resume instead of dropping the whole track.
        ("network-timeout", "15"),
        (
            "stream-lavf-o",
            "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=4",
        ),
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
        ("idle-active", ObserverID::IdleActive),
    ]
}

/// Abstraction over mpv operations for testability. The real implementation
/// wraps libmpv FFI calls.
pub trait MpvPlayer: Send + Sync {
    /// Load a file into the playlist. `options` is an optional mpv-style
    /// comma-separated `key=value` list applied to just this entry.
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

    /// Seconds of demuxer cache buffered ahead of the current playback
    /// position. Used by the prefetch worker to detect when the live
    /// transcode HTTP body has fully drained — Plex enforces a per-client
    /// concurrent-transcode cap, so a prefetch session opened while the
    /// live one is still reading from the network gets cut mid-stream.
    ///
    /// Default `None` covers backends without a libmpv property bridge
    /// (Android ExoPlayer, iOS bridge until it grows the call). Callers
    /// must treat `None` as "unknown" — typically waiting a fixed safety
    /// ceiling instead of polling.
    fn demuxer_cache_time(&self) -> Option<f64> {
        None
    }

    /// Raise mpv's client log level to verbose while the spectrum tap is
    /// installed (its frames are FFmpeg INFO messages, which mpv forwards
    /// at `v`), or restore the configured level afterwards.
    ///
    /// Default no-op for backends that don't host the tap (the mobile
    /// bridges — their libmpv builds lack the analysis filters) and for
    /// test doubles.
    fn set_verbose_log(&self, _enabled: bool) {}
}

/// Thread-safe shutdown flag shared between controller and event loop.
/// Prevents use-after-free when the mpv handle is destroyed while event
/// loop callbacks are still in-flight.
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
        assert!(opts.iter().any(|(k, v)| *k == "vo" && *v == "null"));
        assert!(opts.iter().any(|(k, v)| *k == "vid" && *v == "no"));
        assert!(opts.iter().any(|(k, v)| *k == "gapless-audio" && *v == "yes"));
        assert!(opts.iter().any(|(k, v)| *k == "idle" && *v == "yes"));
        assert!(opts.iter().any(|(k, v)| *k == "keep-open" && *v == "no"));
        // The `af` chain must start empty — EQ populates it on demand.
        assert!(!opts.iter().any(|(k, _)| *k == "af"));
        // Stalled HTTP must error out, not hang forever — required for the
        // file-ended retry path and the stall watchdog to kick in.
        assert!(opts.iter().any(|(k, _)| *k == "network-timeout"));
        assert!(opts
            .iter()
            .any(|(k, v)| *k == "stream-lavf-o" && v.contains("reconnect=1")));
    }

    #[test]
    fn test_observed_properties_complete() {
        let props = observed_properties();
        assert_eq!(props.len(), 5);
        assert!(props.iter().any(|(name, id)| *name == "time-pos" && *id == ObserverID::TimePos));
        assert!(props.iter().any(|(name, id)| *name == "duration" && *id == ObserverID::Duration));
        assert!(props.iter().any(|(name, id)| *name == "pause" && *id == ObserverID::Pause));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "playlist-pos" && *id == ObserverID::PlaylistPos));
        assert!(props
            .iter()
            .any(|(name, id)| *name == "idle-active" && *id == ObserverID::IdleActive));
    }

    #[test]
    fn test_observer_id_values() {
        assert_eq!(ObserverID::TimePos as u64, 1);
        assert_eq!(ObserverID::Duration as u64, 2);
        assert_eq!(ObserverID::Pause as u64, 3);
        assert_eq!(ObserverID::PlaylistPos as u64, 5);
        assert_eq!(ObserverID::IdleActive as u64, 9);
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
        assert!(cb.on_idle_active.is_none());
        assert!(cb.on_file_loaded.is_none());
        assert!(cb.on_file_ended.is_none());
        assert!(cb.on_spectrum_frames.is_none());
    }
}
