//! Background task that watches the player for interrupted playback —
//! stalled streams and recovery holds — and drives connection-verified
//! recovery.
//!
//! Live playback runs entirely inside mpv — the Rust prefetch worker only
//! fires `evaluate_connection` after two consecutive download failures, so
//! when an unreachable host hangs mpv on TCP, nothing on the Rust side
//! notices unless we watch for the stall ourselves. Likewise nothing else
//! polls while a track sits held for recovery: the platform network
//! monitors only fire on *interface* changes, and a tunnel-shaped outage
//! (same interface throughout) produces none.
//!
//! Each cycle asks `ConnectionMonitor::evaluate_connection` for a verdict:
//!
//! - `Failover` / `Lost` — the monitor's callbacks own the response
//!   (player URL swap + reload, or the offline flip).
//! - `Healthy` — the connection is fine but playback is still stuck, and
//!   an unchanged-URI verdict fires no callback. The watchdog owns the
//!   kick: `recover_interrupted_playback` reloads the current track at
//!   position (declining if the user explicitly paused). This is the only
//!   path that revives a dead-socket stream after a silent network flip,
//!   and the only automatic exit from a recovery hold on a remote/cloud
//!   server, whose URI is identical before and after an outage.
//! - `Skipped` — another evaluation is in flight; its callbacks will
//!   handle things. Never treated as healthy.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use ramus_core::plex::connection::EvalOutcome;

use crate::state::AppState;

/// How often to check the player. Cheap — just two atomic loads + a
/// `parking_lot::Mutex` snapshot under the player lock.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Minimum gap between consecutive evaluations the watchdog kicks off. The
/// connection monitor has its own re-entrancy guard, but it'd happily
/// short-circuit on every poll until the in-flight evaluation finishes —
/// this also paces the recovery retry cadence while a hold persists.
const EVAL_COOLDOWN: Duration = Duration::from_secs(20);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_eval: Option<std::time::Instant> = None;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let (player, monitor) = {
                let Some(state) = app.try_state::<AppState>() else {
                    continue;
                };
                if !state.player.needs_connection_recovery() {
                    continue;
                }
                (state.player.clone(), Arc::clone(&state.connection_monitor))
            };

            let now = std::time::Instant::now();
            if let Some(prev) = last_eval {
                if now.duration_since(prev) < EVAL_COOLDOWN {
                    continue;
                }
            }
            last_eval = Some(now);

            log::info!("stall watchdog: playback interrupted, evaluating connection");
            let outcome = monitor.evaluate_connection().await;

            // A healthy verdict fires no monitor callback, so the kick is
            // ours. Re-checks interruption + user pause internally, and its
            // own cooldown coalesces a racing recovered-edge reload.
            if outcome == EvalOutcome::Healthy && player.recover_interrupted_playback() {
                log::info!("stall watchdog: connection healthy, reloaded interrupted track");
                crate::events::emit_playback_buffering(&app, true);
            }
        }
    });
}
