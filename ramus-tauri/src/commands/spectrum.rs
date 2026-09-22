//! `set_spectrum_tap` Tauri command: the frontend's switch for the live
//! focus-mode visualiser.
//!
//! The visualiser is a libavfilter tap in mpv's `--af` chain (see
//! `ramus_core::playback::spectrum_tap`). It costs a steady slice of one
//! core while installed, so the frontend installs it when the visualiser
//! mounts and removes it on unmount. Frames stream back over the
//! `spectrum-frames` event; there is nothing to fetch.

use tauri::State;

use crate::state::AppState;

use super::CmdResult;

/// Install (`true`) or remove (`false`) the spectrum tap.
///
/// A no-op on mobile (the bundled libmpv builds lack the analysis
/// filters) and whenever the user has the visualiser disabled in settings
/// — the setting vetoes the request rather than the frontend having to
/// remember to check it.
#[tauri::command]
pub async fn set_spectrum_tap(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    #[cfg(not(mobile))]
    {
        let disabled = state.settings.read().disable_spectrum;
        let want = enabled && !disabled;
        if state.player.set_spectrum_tap(want) {
            log::info!("spectrum tap {}", if want { "installed" } else { "removed" });
        }
    }
    #[cfg(mobile)]
    let _ = (&state, enabled);
    Ok(())
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
