//! The mpv `af` chain: 10-band parametric equalizer string construction,
//! the live spectrum tap, and the commands that push the composed chain
//! to mpv.
//!
//! Both features are `lavfi` entries in the same chain. Setting `af`
//! rebuilds the filter graph without touching the audio output, so
//! toggling either one mid-track is seamless and gapless playback is
//! unaffected. The tap sits after the equalizer so it measures what the
//! listener hears.

use crate::playback::spectrum_tap::{tap_graph, LevelMapper, SpectrumFrame, TapConfig, TapFrame};

use super::{AudioPlayer, PlayerInner};

/// 10-band EQ center frequencies in Hz.
pub const EQ_FREQUENCIES: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// Build the inner lavfi equalizer graph from gain values (no `lavfi=[…]`
/// wrapper).
///
/// Pairs each gain with the corresponding entry from `EQ_FREQUENCIES`
/// (up to whichever is shorter). Rust's `format!` always uses `.` for
/// decimals. NaN and Inf values are sanitized to 0.0.
pub fn build_eq_graph(bands: &[f32]) -> String {
    EQ_FREQUENCIES
        .iter()
        .zip(bands.iter())
        .map(|(freq, gain)| {
            let g = if gain.is_finite() { *gain } else { 0.0 };
            format!("equalizer=f={freq}:width_type=o:w=1:g={g:.1}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Build an mpv `af` lavfi equalizer filter string from gain values.
pub fn build_eq_filter_string(bands: &[f32]) -> String {
    compose_af(Some(&build_eq_graph(bands)), None)
}

/// Compose the mpv `af` property value from optional inner lavfi graphs:
/// the equalizer first, then the spectrum tap. Each present graph becomes
/// one `lavfi=[…]` entry; the result is empty when neither is present,
/// which `set_audio_filters("")` interprets as "no filters".
///
/// mpv's `[…]` quoting protects the commas and nested pad labels inside
/// each graph from the chain's own comma separator.
pub fn compose_af(eq: Option<&str>, tap: Option<&str>) -> String {
    [eq, tap]
        .into_iter()
        .flatten()
        .filter(|g| !g.is_empty())
        .map(|g| format!("lavfi=[{g}]"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Build the mpv `af` chain string for the given EQ state and optional
/// spectrum tap.
///
/// When neither feature is active, returns an empty string —
/// `set_audio_filters("")` clears anything previously set.
pub fn build_af_string(eq_enabled: bool, bands: &[f32], tap: Option<&TapConfig>) -> String {
    let eq = eq_enabled.then(|| build_eq_graph(bands));
    let tap = tap.map(tap_graph);
    compose_af(eq.as_deref(), tap.as_deref())
}

impl AudioPlayer {
    /// Apply or clear the equalizer. The EQ state is remembered so a later
    /// tap toggle can recompose the chain around it.
    ///
    /// Holds `af_chain` across the state change and the push to mpv so a
    /// concurrent tap toggle can't slip its own chain in between.
    pub fn apply_equalizer(&self, enabled: bool, bands: &[f32]) {
        let _chain = self.af_chain.lock();
        let filter = {
            let mut inner = self.inner.lock();
            inner.eq_enabled = enabled;
            inner.eq_bands = bands.to_vec();
            Self::compose_filters(&inner)
        };
        self.mpv.set_audio_filters(&filter);
    }

    /// Install or remove the live spectrum tap. Returns whether the state
    /// actually changed (a repeated request is a no-op that leaves mpv
    /// untouched).
    ///
    /// The tap's frames ride mpv's verbose log stream, so the log level is
    /// raised before the graph is installed and lowered only after it has
    /// been removed. Enabling also resets the level mapper so the previous
    /// session's running peak doesn't shape the first frames.
    ///
    /// The state change, the log-level move and the chain push happen
    /// under `af_chain` as one step: overlapping toggles run one after the
    /// other, so mpv always ends up matching the last state (see the
    /// concurrent-toggle test).
    pub fn set_spectrum_tap(&self, enabled: bool) -> bool {
        let _chain = self.af_chain.lock();
        let filter = {
            let mut inner = self.inner.lock();
            if inner.spectrum_tap_enabled == enabled {
                return false;
            }
            inner.spectrum_tap_enabled = enabled;
            inner.tap_awaiting_first_batch = enabled;
            if enabled {
                inner.tap_mapper.reset();
            }
            Self::compose_filters(&inner)
        };
        if enabled {
            self.mpv.set_verbose_log(true);
        }
        self.mpv.set_audio_filters(&filter);
        if !enabled {
            self.mpv.set_verbose_log(false);
        }
        true
    }

    /// Whether the spectrum tap is currently part of the chain.
    pub fn spectrum_tap_enabled(&self) -> bool {
        self.inner.lock().spectrum_tap_enabled
    }

    /// Map a raw tap timestamp onto the track's timeline.
    ///
    /// `pts_time` is mpv's own timeline for the current stream, which is
    /// exactly what the `time-pos` observer reports raw. After a transcode
    /// `offset=` resume that stream is 0-based and `position_base` holds
    /// the shift; the frontend's position is the shifted value, so frames
    /// must be shifted the same way or the visualiser drifts for the rest
    /// of that stream.
    pub fn tap_frame_position(&self, raw_pts: f64) -> f64 {
        raw_pts + self.inner.lock().position_base
    }

    /// Turn a batch of parsed tap frames into IPC-ready frames: timeline
    /// remap plus dB → bar-height quantisation through the running-peak
    /// mapper. One lock for the whole batch.
    pub fn map_tap_frames(&self, frames: Vec<TapFrame>) -> Vec<SpectrumFrame> {
        let mut inner = self.inner.lock();
        if inner.tap_awaiting_first_batch {
            if let Some(first) = frames.first() {
                log::info!(
                    "spectrum tap: first frames arrived ({} in batch, {} bands, pts {:.3})",
                    frames.len(),
                    first.db.len(),
                    first.pts
                );
                inner.tap_awaiting_first_batch = false;
            }
        }
        let base = inner.position_base;
        frames
            .into_iter()
            .map(|f| SpectrumFrame {
                pos: f.pts + base,
                bands: inner.tap_mapper.map(&f.db),
            })
            .collect()
    }

    /// Compose the `af` value from the remembered EQ state and tap flag.
    fn compose_filters(inner: &PlayerInner) -> String {
        let tap = inner.spectrum_tap_enabled.then(TapConfig::default);
        build_af_string(inner.eq_enabled, &inner.eq_bands, tap.as_ref())
    }
}

/// The mapper the player owns per tap session; re-exported type alias so
/// callers constructing a player know which frame rate it assumes.
pub(super) fn new_tap_mapper() -> LevelMapper {
    LevelMapper::new(TapConfig::default().fps)
}
