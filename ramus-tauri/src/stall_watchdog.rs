//! Background task that watches the player for stalled playback and
//! triggers a connection re-evaluation when it sees one.
//!
//! Plex's HLS transcode path runs entirely inside mpv — the Rust prefetch
//! worker only fires `evaluate_connection` after two consecutive download
//! failures, and `next_uncached_target_in_lookahead` deliberately skips
//! transcode tracks. So when an unreachable host hangs mpv on TCP, nothing
//! on the Rust side notices unless we watch for the stall ourselves.
//!
//! `AudioPlayer::is_stalled` returns `true` when the player believes it
//! should be `Playing` but no `time-pos` event has arrived for
//! `STALL_THRESHOLD_SECS`. This task polls every few seconds; when it sees
//! a stall it asks `ConnectionMonitor::evaluate_connection` to test the
//! current URL and, if dead, swap to a remote / relay connection. The
//! re-evaluation cooldown is enforced inside the monitor (it short-circuits
//! while another evaluation is in flight), so polling more often is cheap.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// How often to check the player. Cheap — just two atomic loads + a
/// `parking_lot::Mutex` snapshot under the player lock.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Minimum gap between consecutive evaluations the watchdog kicks off. The
/// connection monitor has its own re-entrancy guard, but it'd happily
/// short-circuit on every poll until the in-flight evaluation finishes —
/// this just avoids spamming `info!` logs.
const EVAL_COOLDOWN: Duration = Duration::from_secs(20);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_eval: Option<std::time::Instant> = None;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            if !state.player.is_stalled() {
                continue;
            }

            let now = std::time::Instant::now();
            if let Some(prev) = last_eval {
                if now.duration_since(prev) < EVAL_COOLDOWN {
                    continue;
                }
            }
            last_eval = Some(now);

            log::info!("stall watchdog: no playback progress, re-evaluating connection");
            let monitor = Arc::clone(&state.connection_monitor);
            // Use `tauri::async_runtime::spawn` (not raw `tokio::spawn`) for
            // consistency with every other `evaluate_connection` call site.
            // Works either way today since Tauri is Tokio-backed, but avoids
            // a panic if Tauri ever swaps executors.
            tauri::async_runtime::spawn(async move {
                monitor.evaluate_connection().await;
            });
        }
    });
}
