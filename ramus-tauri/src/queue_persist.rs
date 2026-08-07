//! Persists the playback queue so a restart resumes where the last session
//! left off.
//!
//! Two writers, deliberately split by what they react to:
//!
//! - **Structural changes** (a new queue, an append/insert/remove, a track
//!   advance) write immediately via [`save_soon`]. These are rare and
//!   user-driven, and getting them wrong loses the whole queue.
//! - **Position** rides a slow ticker and writes only `queue_store`'s small
//!   position file, never the track list. Doing it from `on_position_change`
//!   instead would put a serialising write on the mpv event-loop thread ~30
//!   times a second, and that thread must stay clear (the Android bridge
//!   rules exist for the same reason). The ticker also gates on real
//!   progress, so a paused or stopped app writes nothing at all.
//!
//! Cost of the ticker approach is bounded imprecision: a hard kill (mobile
//! OOM, force-quit) loses at most [`POLL_INTERVAL`] of position. Clean exits
//! flush explicitly — [`save_blocking`] runs from `ExitRequested` on desktop
//! and from the `flush_queue_state` command when a mobile webview backgrounds.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use ramus_core::models::Track;
use ramus_core::models::PlaybackStatus;
use ramus_core::playback::queue_store;

use crate::state::AppState;

/// How often to consider persisting the playing position.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Position drift that justifies a rewrite. Below this the snapshot on disk
/// is close enough that rewriting it is pure flash wear.
const POSITION_DELTA_SECS: f64 = 5.0;

/// Snapshot the queue under the player lock.
///
/// `None` means "don't write at all" — no app state yet (startup races), or
/// the user has turned resume off. Distinct from an *empty* queue, which is a
/// real state meaning "erase what's on disk".
fn snapshot(app: &AppHandle) -> Option<(Vec<Track>, usize, f64)> {
    let state = app.try_state::<AppState>()?;
    if !state.settings.read().resume_queue_on_launch {
        return None;
    }
    let ps = state.player.state();
    Some((ps.queue, ps.queue_index, state.player.position()))
}

/// Write the queue synchronously on the calling thread.
///
/// Use from the flush paths, where the process may be suspended or torn down
/// the moment we return and a spawned task would never run.
pub fn save_blocking(app: &AppHandle) {
    let Some((tracks, index, position)) = snapshot(app) else {
        return;
    };
    if tracks.is_empty() {
        queue_store::clear();
        return;
    }
    if let Err(e) = queue_store::save(&tracks, index, position) {
        log::warn!("queue_persist: save failed: {e}");
    }
}

/// Write the queue off the calling thread.
///
/// Safe to call from mpv event callbacks: it hands the work to the Tauri
/// async runtime rather than serialising a few hundred KB inline (and uses
/// `tauri::async_runtime::spawn`, not `tokio::spawn` — mpv callbacks run on
/// the libmpv event-loop thread, which has no tokio reactor).
pub fn save_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        save_blocking(&app);
    });
}

/// Forget the stored queue. Called when the queue is cleared and on sign-out
/// — one account's queue must never surface under the next.
pub fn forget() {
    queue_store::clear();
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_written_pos: Option<f64> = None;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            if !state.settings.read().resume_queue_on_launch {
                continue;
            }

            // Deliberately NOT `player.state()`: that deep-clones the whole
            // queue, which at the persisted cap is hundreds of KB copied
            // every tick to read two values.
            if state.player.status() != PlaybackStatus::Playing {
                // Only a *playing* queue accumulates position worth
                // recording. A paused or stopped player's position was
                // already written by whichever change put it there.
                continue;
            }
            let Some(rating_key) = state.player.current_track_key() else {
                continue;
            };
            let position = state.player.position();

            // Absolute difference so a backward seek counts too.
            if last_written_pos.is_some_and(|p| (position - p).abs() < POSITION_DELTA_SECS) {
                continue;
            }

            // Position only — the track list is untouched until something
            // structural changes it.
            if let Err(e) = queue_store::save_position(&rating_key, position) {
                log::warn!("queue_persist: periodic position save failed: {e}");
                continue;
            }
            last_written_pos = Some(position);
        }
    });
}
