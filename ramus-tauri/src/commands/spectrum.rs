//! `get_spectrum` Tauri command: the frontend's handle on per-track
//! spectrograms produced by `spectrum_analyzer.rs`.
//!
//! Two outcomes, in decision order:
//!
//! 1. **Cached file + `.spec` sibling** → return whatever the `.spec` says
//!    (`Ready` with frames, or `Unavailable` if the analyser previously
//!    recorded a decoder failure for this specific file).
//! 2. **No cached file yet** → `Analysing`. The prefetch worker downloads
//!    every track (direct-play and transcoded) into the cache and the
//!    analyser runs against the on-disk file; `spectrum-ready` fires when
//!    the `.spec` lands and the frontend re-invokes this command. Both
//!    direct-play and Ogg/Opus transcoded sources are decodable.
//!
//! A full `SpectrumState::Ready` is ~500 KB–2 MB depending on track length.
//! The JSON IPC cost is accepted; switch to an asset-protocol read of the
//! `.spec` file if it profiles slow.

use tauri::State;

use ramus_core::playback::spectrum::{read_spec_file, SpectrumState};

use crate::state::AppState;

use super::CmdResult;

/// Return the cached spectrum state for a track. Never blocks on analysis —
/// if no spectrogram exists yet, returns `Analysing` so the frontend shows
/// the right placeholder while the prefetch worker downloads + analyses
/// the file.
#[tauri::command]
pub async fn get_spectrum(
    state: State<'_, AppState>,
    rating_key: String,
) -> CmdResult<SpectrumState> {
    if state.settings.read().disable_spectrum {
        return Ok(SpectrumState::Unavailable {
            reason: "disabled".into(),
        });
    }

    // Cached file → trust whatever the .spec says (Analysing if the
    // analyser hasn't written one yet). Check the persistent downloads
    // map first so downloaded tracks resolve to their permanent .spec,
    // then fall back to the LRU prefetch cache.
    let audio_path = state
        .player
        .persistent_download_paths()
        .get(&rating_key)
        .cloned()
        .or_else(|| {
            state
                .player
                .with_cache(|cache| cache.get(&rating_key).map(|p| p.to_path_buf()))
        });
    if let Some(audio_path) = audio_path {
        return Ok(read_spec_file(&audio_path).unwrap_or(SpectrumState::Analysing));
    }

    // No cached file yet → the prefetch worker is still pulling it (or
    // waiting for the live transcode session to drain before opening its
    // own). Either way, frontend gets the analysing placeholder.
    Ok(SpectrumState::Analysing)
}
