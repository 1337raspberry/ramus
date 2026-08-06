//! Background task that watches the player for interrupted playback —
//! stalled streams and recovery holds — and drives connection-verified
//! recovery. It also owns the adaptive quality check, being the task that
//! already polls the player on a steady cadence; that check runs on every
//! poll rather than behind the recovery gate below, because a link that is
//! merely too slow produces many short rebuffers instead of one long silence
//! and often never counts as "interrupted" at all.
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
//!
//! A healthy verdict is not on its own a reason to reload. A link that is
//! merely *too slow* for the current stream answers a `/identity` probe
//! happily while mpv rebuffers, and reloading there is actively harmful: the
//! URL re-resolves to the same stream, the demuxer cache is discarded, and the
//! re-open has to re-earn those bytes over the link that was already short of
//! bandwidth. Worse, the reload does not reset the position-tick clock, so if
//! the fresh open takes longer than `EVAL_COOLDOWN` to produce its first tick,
//! the next cycle tears it down and starts again — a stream that can never
//! establish. So before reloading, the watchdog asks whether the source is
//! still arriving; if it is, the diagnosis is starvation, and the right move
//! is to leave mpv alone to buffer through it.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use ramus_core::models::PlaybackMode;
use ramus_core::playback::player::AudioPlayer;
use ramus_core::plex::connection::EvalOutcome;

use crate::events::PlaybackQualityPayload;
use crate::state::AppState;

/// How often to check the player. Cheap — just two atomic loads + a
/// `parking_lot::Mutex` snapshot under the player lock.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Minimum gap between consecutive evaluations the watchdog kicks off. The
/// connection monitor has its own re-entrancy guard, but it'd happily
/// short-circuit on every poll until the in-flight evaluation finishes —
/// this also paces the recovery retry cadence while a hold persists.
const EVAL_COOLDOWN: Duration = Duration::from_secs(20);

/// Gap between the two `demuxer-cache-time` samples that decide whether the
/// source is still arriving. Long enough that even a trickle registers,
/// short enough not to delay a genuine recovery reload noticeably.
const SOURCE_PROBE_INTERVAL: Duration = Duration::from_millis(1200);

/// Cache-time growth counted as "still arriving". Matches the prefetch
/// drain gate's tolerance — mpv's reported value wobbles fractionally even
/// when nothing new is being pulled.
const SOURCE_PROBE_EPSILON: f64 = 0.05;

/// Whether mpv's demuxer cache is still growing, i.e. bytes are landing
/// however slowly.
///
/// This is what separates a starving stream from a dead socket, which look
/// identical from the outside once the position ticks stop. `demuxer-cache-time`
/// is bridged on all three platforms (it is what the prefetch drain gate
/// uses), so this needs no new mpv plumbing.
///
/// Returns `false` when the property is unavailable — the safe default is
/// the pre-existing behaviour, a reload.
pub(crate) async fn source_still_arriving(player: &AudioPlayer) -> bool {
    let Some(before) = player.demuxer_cache_time() else {
        return false;
    };
    tokio::time::sleep(SOURCE_PROBE_INTERVAL).await;
    let Some(after) = player.demuxer_cache_time() else {
        return false;
    };
    after - before > SOURCE_PROBE_EPSILON
}

/// Take an adaptive quality step if the link warrants one, and report the
/// resulting state to the frontend when it changes.
///
/// The player owns the decision (cooldown, mode eligibility, ladder floor and
/// whether to apply mid-track); this only drives it and reports. Emitting on
/// change rather than every poll keeps a steady starving link from spamming
/// the event bus at 0.2 Hz for the length of a track.
fn adapt_quality(
    app: &AppHandle,
    player: &AudioPlayer,
    mode: PlaybackMode,
    last: &mut Option<PlaybackQualityPayload>,
) {
    if player.is_starving() {
        if let Some(step) = player.consider_bandwidth_degrade() {
            log::info!(
                "quality: stepped to {} kbps (applied to current track: {})",
                step.bitrate.as_kbps(),
                step.applied_to_current,
            );
            if step.applied_to_current {
                // The step swaps the stream, so the same gap-covering the
                // recovery reload gets: tell the UI it's buffering, and hold
                // the process awake through the silent window on iOS.
                crate::events::emit_playback_buffering(app, true);
                if let Some(state) = app.try_state::<AppState>() {
                    crate::set_recovery_grace(app, &state.recovery_grace, true);
                }
            }
        }
    }

    // Read back *after* any step: a successful one clears the evidence it
    // acted on, so the report becomes "playing at N kbps" rather than
    // "starving", which is what the user should see.
    let payload = PlaybackQualityPayload {
        starving: player.is_starving(),
        degraded_to_kbps: player.bandwidth_degrade().map(|b| b.as_kbps()),
        adaptation_blocked: !mode.adapts_to_slow_connection(),
    };
    if last.as_ref() != Some(&payload) {
        crate::events::emit_playback_quality(app, payload.clone());
        *last = Some(payload);
    }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_eval: Option<std::time::Instant> = None;
        let mut last_quality: Option<PlaybackQualityPayload> = None;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let (player, monitor, mode) = {
                let Some(state) = app.try_state::<AppState>() else {
                    continue;
                };
                let mode = state.settings.read().playback_mode;
                (
                    state.player.clone(),
                    Arc::clone(&state.connection_monitor),
                    mode,
                )
            };

            // Adaptive quality runs on every poll, independent of the
            // recovery ladder below. A starving link produces many short
            // rebuffers rather than one long silence, so it frequently never
            // trips `needs_connection_recovery` at all.
            adapt_quality(&app, &player, mode, &mut last_quality);

            if !player.needs_connection_recovery() {
                continue;
            }

            let now = std::time::Instant::now();
            if let Some(prev) = last_eval {
                if now.duration_since(prev) < EVAL_COOLDOWN {
                    continue;
                }
            }
            last_eval = Some(now);

            log::info!("stall watchdog: playback interrupted, evaluating connection");
            let outcome = monitor.evaluate_connection().await;

            // A slow link probes healthy while mpv rebuffers. Confirm the
            // source has actually stopped arriving before reloading — the
            // starvation verdict is the cheap pattern check (no IPC), the
            // cache-time probe is the authority on whether bytes are landing
            // right now, which also covers a slow link that has since died.
            if outcome == EvalOutcome::Healthy
                && player.is_starving()
                && source_still_arriving(&player).await
            {
                log::info!(
                    "stall watchdog: connection healthy and source still arriving — stream is starving, not stuck; leaving mpv to rebuffer"
                );
                continue;
            }

            // A healthy verdict fires no monitor callback, so the kick is
            // ours. Re-checks interruption + user pause internally, and its
            // own cooldown coalesces a racing recovered-edge reload.
            if outcome == EvalOutcome::Healthy && player.recover_interrupted_playback() {
                log::info!("stall watchdog: connection healthy, reloaded interrupted track");
                crate::events::emit_playback_buffering(&app, true);
                // Keep the process awake through the silent reload window
                // (iOS background-task assertion; no-op elsewhere). The
                // first position tick releases it.
                if let Some(state) = app.try_state::<AppState>() {
                    crate::set_recovery_grace(&app, &state.recovery_grace, true);
                }
            }
        }
    });
}
