//! `set_spectrum_tap` Tauri command: the frontend's switch for the live
//! focus-mode visualiser.
//!
//! The visualiser is a libavfilter tap in mpv's `--af` chain (see
//! `ramus_core::playback::spectrum_tap`). It costs a steady slice of one
//! core while installed, so the frontend installs it when the visualiser
//! mounts and removes it on unmount. Frames stream back over the
//! `spectrum-frames` event; the one thing to fetch is the frame layout
//! (`get_spectrum_layout`), which never changes while the app runs.

use ramus_core::playback::spectrum_tap::{band_onset_delays, TapConfig};
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

use super::CmdResult;

/// Install (`true`) or remove (`false`) the spectrum tap.
///
/// A no-op on Android (its libmpv build lacks the analysis filters) and
/// whenever the user has the visualiser disabled in settings — the
/// setting vetoes the request rather than the frontend having to remember
/// to check it.
#[tauri::command]
pub async fn set_spectrum_tap(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    #[cfg(not(target_os = "android"))]
    {
        let disabled = state.settings.read().disable_spectrum;
        let want = enabled && !disabled;
        if state.player.set_spectrum_tap(want) {
            log::info!("spectrum tap {}", if want { "installed" } else { "removed" });
        }
    }
    #[cfg(target_os = "android")]
    let _ = (&state, enabled);
    Ok(())
}

/// The shape of every `spectrum-frames` frame and how to read it in step
/// with the audio.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumLayout {
    /// Bands per channel in every frame.
    pub band_count: u32,
    /// Frames per second the tap emits.
    pub fps: u32,
    /// How long each band's level trails the audio it measures, in
    /// seconds, the lowest band first (`spectrum_tap::band_onset_delays`):
    /// read a band this much further on and its onsets line up with the
    /// treble's.
    pub onset_delays: Vec<f32>,
}

/// The tap's frame layout. Fixed for the life of the app, so the frontend
/// fetches it once.
#[tauri::command]
pub fn get_spectrum_layout() -> SpectrumLayout {
    let cfg = TapConfig::default().normalised();
    SpectrumLayout {
        band_count: cfg.bands as u32,
        fps: cfg.fps,
        onset_delays: band_onset_delays(&cfg),
    }
}

/// Change the spectral tilt applied to every band before it is mapped to
/// a bar height, in dB per octave. A development tuning control; the
/// shipped value lives in `ramus-core`'s `TILT_DB_PER_OCTAVE`, and a
/// release build refuses the call so the IPC surface carries no live
/// tuning knob.
#[tauri::command]
pub async fn set_spectrum_tilt(state: State<'_, AppState>, db_per_octave: f32) -> CmdResult<()> {
    #[cfg(all(not(mobile), debug_assertions))]
    {
        state.player.set_tap_tilt(db_per_octave);
        Ok(())
    }
    #[cfg(not(all(not(mobile), debug_assertions)))]
    {
        let _ = (&state, db_per_octave);
        Err("set_spectrum_tilt is a development-only tuning control".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_matches_the_frames_the_tap_emits() {
        let layout = get_spectrum_layout();
        let cfg = TapConfig::default().normalised();
        assert_eq!(layout.band_count as usize, cfg.bands);
        assert_eq!(layout.fps, cfg.fps);
        assert_eq!(layout.onset_delays.len(), cfg.bands);
    }
}
