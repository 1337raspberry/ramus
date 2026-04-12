//! `get_spectrum` Tauri command — the frontend's handle on per-track
//! spectrograms produced by `spectrum_analyzer.rs`.
//!
//! Three possible outcomes, in decision order:
//!
//! 1. **Cached file + `.spec` sibling** → return whatever the `.spec`
//!    says (`Ready` with the frames, or `Unavailable` if the analyser
//!    previously recorded a decoder failure).
//! 2. **No cached file** AND **track would be transcoded** →
//!    `Unavailable { reason: "transcoding" }`. Transcoded tracks stream
//!    via HLS, which symphonia can't decode, so we surface this
//!    immediately instead of leaving the user on a forever-analysing
//!    placeholder.
//! 3. **Otherwise** → `Analysing`. The prefetch path (or the fast path
//!    for the current track) will emit `spectrum-ready` when it finishes
//!    and the frontend will re-invoke this command to pick up the result.
//!
//! Size note: a full `SpectrumState::Ready` is ~500 KB–2 MB depending on
//! track length. We accept the JSON IPC cost for now; if it profiles as
//! slow, we'll switch to an asset-protocol read of the `.spec` file.

use tauri::State;

use ramus_core::playback::spectrum::{read_spec_file, SpectrumState};

use crate::state::AppState;

use super::CmdResult;

/// Return the cached spectrum state for the given track. Never blocks on
/// analysis — if no spectrogram exists yet, returns either `Analysing`
/// (analyser pending) or `Unavailable` (track will be transcoded and
/// can't be analysed at all) so the frontend shows the right placeholder.
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

    // 1. Cached file → trust whatever the .spec says (or Analysing if
    //    the analyser hasn't written one yet).
    let audio_path = state
        .player
        .with_cache(|cache| cache.get(&rating_key).map(|p| p.to_path_buf()));
    if let Some(audio_path) = audio_path {
        return Ok(read_spec_file(&audio_path).unwrap_or(SpectrumState::Analysing));
    }

    // 2. No cached file. Will this track ever be analysable? Transcoded
    //    tracks bypass the prefetch/analyser pipeline entirely, so the
    //    correct placeholder is "unavailable", not "analysing".
    if state.player.would_transcode(&rating_key) {
        return Ok(SpectrumState::Unavailable {
            reason: "transcoding".into(),
        });
    }

    // 3. Direct-play track, not yet cached — analyser is in flight.
    Ok(SpectrumState::Analysing)
}
