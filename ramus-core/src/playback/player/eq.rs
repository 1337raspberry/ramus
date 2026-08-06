//! 10-band parametric equalizer: lavfi filter-string construction and the
//! command that pushes it to mpv.

use super::AudioPlayer;

/// 10-band EQ center frequencies in Hz.
pub const EQ_FREQUENCIES: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// Build an mpv `af` lavfi equalizer filter string from gain values.
///
/// Pairs each gain with the corresponding entry from `EQ_FREQUENCIES`
/// (up to whichever is shorter). Rust's `format!` always uses `.` for
/// decimals. NaN and Inf values are sanitized to 0.0.
pub fn build_eq_filter_string(bands: &[f32]) -> String {
    let filters: Vec<String> = EQ_FREQUENCIES
        .iter()
        .zip(bands.iter())
        .map(|(freq, gain)| {
            let g = if gain.is_finite() { *gain } else { 0.0 };
            format!("equalizer=f={freq}:width_type=o:w=1:g={g:.1}")
        })
        .collect();

    format!("lavfi=[{}]", filters.join(","))
}

/// Build the mpv `af` chain string for the current EQ state.
///
/// When EQ is enabled, returns the lavfi equalizer chain. When disabled,
/// returns an empty string — `set_audio_filters("")` interprets this as
/// "no filters", clearing anything previously set.
pub fn build_af_string(eq_enabled: bool, bands: &[f32]) -> String {
    if eq_enabled {
        build_eq_filter_string(bands)
    } else {
        String::new()
    }
}

impl AudioPlayer {
    /// Apply or clear the equalizer. When `enabled` is false the `af`
    /// chain is cleared entirely.
    pub fn apply_equalizer(&self, enabled: bool, bands: &[f32]) {
        let filter = build_af_string(enabled, bands);
        self.mpv.set_audio_filters(&filter);
    }
}
