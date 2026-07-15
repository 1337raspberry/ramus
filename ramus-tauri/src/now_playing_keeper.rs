//! Keeps the OS "now playing" scrubber honest during mid-track stalls.
//!
//! The lock screen / Control Center (and the desktop souvlaki surface)
//! extrapolate the playback position from the last `(position, rate)` we hand
//! them — so when audio stalls mid-track (a network hiccup, or the gap while
//! connection failover reloads the stream) the scrubber keeps sailing forward
//! as if still playing, ending up seconds ahead of the truth.
//!
//! Our event callbacks can't fix this on their own: no `time-pos` events fire
//! *during* a stall, and the speculative freeze pushed at recovery time gets
//! undone almost immediately — by the reload's first (pre-audio) position tick
//! re-anchoring, or by a metadata refresh that carries `is_playing = true`.
//!
//! This task polls the player. While it believes playback is progressing it
//! does nothing (the OS extrapolation is already correct). The moment progress
//! stalls it pins the scrubber to the true position at rate 0, re-asserting on
//! every tick so a stray rate-1 push can't let it drift, and re-syncs to
//! rate 1 once audio flows again. It mirrors the frontend buffering watchdog,
//! but for the OS surface. Cross-platform — the same drift affects desktop
//! souvlaki, iOS, and Android, and all three share this `MediaKeyHandler`.

use std::time::Duration;

use ramus_core::playback::media_keys::MediaKeyHandler;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// How often to check the player. Cheap — one `parking_lot::Mutex` snapshot.
/// Sub-second so a one-shot rate-1 push (the reload's first pre-audio tick, a
/// metadata refresh) is re-frozen before the scrubber visibly drifts.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// No position tick for this long (while Playing, after at least one tick) means
/// audio has stalled and the OS scrubber must be frozen. Kept a touch above the
/// frontend buffering threshold so the lock screen doesn't stutter on
/// sub-second micro-hiccups that recover on their own.
const STALL_FREEZE_THRESHOLD: Duration = Duration::from_millis(2000);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Whether we're currently holding the scrubber frozen. Only used to
        // fire the resume re-sync once, on the stall→flowing edge.
        let mut os_frozen = false;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let snap = state
                .player
                .media_position_snapshot(STALL_FREEZE_THRESHOLD);

            if !snap.is_playing {
                // Paused/stopped: the pause/stop event pushes own the OS
                // surface; stand down and forget any freeze we were holding.
                os_frozen = false;
                continue;
            }

            let guard = state.media_controls.lock();
            let Some(ref mc) = *guard else {
                continue;
            };

            if snap.progress_stalled {
                // Re-assert every tick: a stray rate-1 push (the reload's first
                // pre-audio position tick, or a metadata refresh) would
                // otherwise let the scrubber drift until the next stall.
                mc.update_playback_state(false, snap.position);
                os_frozen = true;
            } else if os_frozen {
                // Audio is flowing again — hand the OS the true position at
                // rate 1 so it resumes extrapolating from the truth, not from
                // wherever it had frozen.
                mc.update_playback_state(true, snap.position);
                os_frozen = false;
            }
        }
    });
}
