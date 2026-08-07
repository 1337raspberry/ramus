use tauri::{AppHandle, State};

use ramus_core::models::Track;
use ramus_core::playback::lyrics::{self, LyricsFetchResult};
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::waveform;
#[cfg(target_os = "android")]
use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;

use crate::events::{emit_playback_state, PlaybackStatePayload};
use crate::state::AppState;

use super::CmdResult;

#[tauri::command]
pub async fn play_tracks(
    app: AppHandle,
    state: State<'_, AppState>,
    tracks: Vec<Track>,
    start_at: usize,
) -> CmdResult<()> {
    // Close the outgoing session before loading the new queue, while the
    // player still holds the old track at its true position — scrobbling it
    // if it crossed the threshold. `next` stays None: the new queue's
    // track_started below must carry the session id load_queue rotates.
    let prev = state.player.state().current_track.clone();
    if let Some(ref prev) = prev {
        let pos = state.player.position();
        let dur = state.player.duration();
        let sid = state.player.play_session_id();
        state
            .session_reporter
            .track_transition(prev, pos, dur, None, &sid);
    } else {
        state.session_reporter.playback_stopped();
    }

    // Abort in-flight prefetch from the previous album — the new queue has a
    // different lookahead window. The worker starts a fresh cycle on the next
    // playlist-pos-change from mpv.
    state.prefetch_handle.notify_cancel();

    state.player.load_queue(tracks, start_at);

    let player_state = state.player.state();
    emit_playback_state(
        &app,
        PlaybackStatePayload {
            status: "playing".to_string(),
            current_track: player_state.current_track.clone(),
            queue_index: player_state.queue_index,
        },
    );

    // Authoritative track_started call. The mpv on_playlist_pos_change callback
    // may not fire when the new queue also starts at index 0 (playlist-pos
    // doesn't change).
    if let Some(ref track) = player_state.current_track {
        state
            .session_reporter
            .track_started(track, &state.player.play_session_id());

        // Push metadata to OS media controls. Duration is 0 until
        // on_duration_change fires and re-pushes with the real value.
        if let Some(ref mc) = *state.media_controls.lock() {
            let meta = MediaMetadata::from_track(track, 0.0, track.duration, true);
            mc.update_metadata(&meta);
        }
    }

    // Kick off an initial prefetch cycle for the freshly-loaded queue. If the
    // mpv playlist-pos callback also fires natural-advance, the worker
    // coalesces (only starts a new cycle when idle).
    state.prefetch_handle.notify_natural_advance();

    crate::queue_persist::save_soon(&app);

    Ok(())
}

/// Open the Plex session and push OS metadata for a queue that has just been
/// materialised from a restored snapshot.
///
/// A materialisation reaches `load_queue_at` directly, bypassing
/// `play_tracks`, so this is the only thing that tells Plex (and the lock
/// screen) that playback started. The playlist-pos callback can't: it reports
/// from the player's transition snapshot, which a fresh queue load clears.
///
/// No-op unless a materialisation actually ran, so every transport command
/// below can call it unconditionally.
fn report_if_materialised(app: &AppHandle, state: &AppState) {
    if !state.player.take_just_materialized() {
        return;
    }
    let ps = state.player.state();
    let Some(ref track) = ps.current_track else {
        return;
    };
    let playing = ps.status == ramus_core::models::PlaybackStatus::Playing;

    state
        .session_reporter
        .track_started(track, &state.player.play_session_id());

    if let Some(ref mc) = *state.media_controls.lock() {
        // Restore deliberately leaves the OS widget empty, so this is the
        // first metadata the platform sees for this track.
        let meta = MediaMetadata::from_track(track, 0.0, track.duration, playing);
        mc.update_metadata(&meta);
    }

    emit_playback_state(
        app,
        PlaybackStatePayload {
            status: format!("{:?}", ps.status).to_lowercase(),
            current_track: ps.current_track.clone(),
            queue_index: ps.queue_index,
        },
    );

    // The restored queue's lookahead window is new to the worker — it has
    // been idle since launch.
    state.prefetch_handle.notify_natural_advance();
}

#[tauri::command]
pub async fn toggle_play_pause(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.player.toggle_play_pause();
    report_if_materialised(&app, &state);
    Ok(())
}

// The queue-moving and queue-editing commands below all take an `AppHandle`
// solely to persist the resulting queue. Tauri injects it, so the JS invokes
// are unchanged. They save explicitly rather than relying on the mpv
// playlist-pos callback: materialising a restored queue at index 0 issues no
// `playlist_play_index`, so no pos-change event follows.

#[tauri::command]
pub async fn next_track(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.next();
    report_if_materialised(&app, &state);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

#[tauri::command]
pub async fn previous_track(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.previous();
    report_if_materialised(&app, &state);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

#[tauri::command]
pub async fn seek(app: AppHandle, state: State<'_, AppState>, position: f64) -> CmdResult<()> {
    state.player.seek(position);
    // Scrubbing a restored track materialises it — that is the play intent,
    // so the session opens here rather than on a later transport command.
    report_if_materialised(&app, &state);
    state.session_reporter.playback_seeked(position);
    // Report new position to OS media controls so the scrubber jumps.
    if let Some(ref mc) = *state.media_controls.lock() {
        let is_playing = state.player.state().status == ramus_core::models::PlaybackStatus::Playing;
        mc.update_playback_state(is_playing, position);
    }
    Ok(())
}

#[tauri::command]
pub async fn set_volume(state: State<'_, AppState>, volume: f64) -> CmdResult<()> {
    state.player.set_volume(volume);
    Ok(())
}

#[tauri::command]
pub async fn get_volume(state: State<'_, AppState>) -> CmdResult<f64> {
    Ok(state.player.volume())
}

#[tauri::command]
pub async fn append_to_queue(
    app: AppHandle,
    state: State<'_, AppState>,
    tracks: Vec<Track>,
) -> CmdResult<()> {
    state.player.append_to_queue(tracks);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

#[tauri::command]
pub async fn insert_next(
    app: AppHandle,
    state: State<'_, AppState>,
    tracks: Vec<Track>,
) -> CmdResult<()> {
    state.player.insert_next(tracks);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

#[tauri::command]
pub async fn remove_from_queue(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
) -> CmdResult<()> {
    state.player.remove_from_queue(index);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

#[tauri::command]
pub async fn jump_to_queue_index(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.jump_to_index(index);
    report_if_materialised(&app, &state);
    crate::queue_persist::save_soon(&app);
    Ok(())
}

/// Write the queue snapshot to disk immediately.
///
/// The frontend calls this when a mobile webview backgrounds: the process may
/// be suspended moments later, freezing the periodic writer mid-interval.
#[tauri::command]
pub async fn flush_queue_state(app: AppHandle) -> CmdResult<()> {
    crate::queue_persist::save_blocking(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_queue(state: State<'_, AppState>) -> CmdResult<Vec<Track>> {
    Ok(state.player.state().queue)
}

/// Stop playback and empty the queue.
///
/// Deliberately self-sufficient rather than leaning on the mpv idle callback
/// for the stopped teardown. Two reasons:
///
/// 1. `AudioPlayer::stop` clears `current_track` and the pending-transition
///    snapshot before mpv goes idle, so by the time the idle handler runs it
///    has nothing left to close the outgoing Plex session with — the final
///    track's stopped-at-position report and any boundary scrobble would be
///    lost. Close it out here first, while the player still holds its
///    pre-stop state.
/// 2. Android suppresses `mpvIdleActive` while the player carries an error,
///    so clearing a queue whose current track failed to load would never
///    reach the teardown at all.
///
/// The idle callback still runs on the platforms that emit it; everything it
/// repeats (stopped report with no track, controls clear, prefetch cancel) is
/// idempotent.
#[tauri::command]
pub async fn clear_queue(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let prev = state.player.state().current_track.clone();
    if let Some(ref prev) = prev {
        let pos = state.player.position();
        let dur = state.player.duration();
        let sid = state.player.play_session_id();
        state
            .session_reporter
            .track_transition(prev, pos, dur, None, &sid);
    } else {
        state.session_reporter.playback_stopped();
    }

    state.prefetch_handle.notify_cancel();
    state.player.stop();
    crate::queue_persist::forget();

    if let Some(ref mc) = *state.media_controls.lock() {
        mc.clear();
    }

    emit_playback_state(
        &app,
        PlaybackStatePayload {
            status: "stopped".to_string(),
            current_track: None,
            queue_index: 0,
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn apply_equalizer(
    state: State<'_, AppState>,
    enabled: bool,
    bands: Vec<f32>,
) -> CmdResult<()> {
    state.player.apply_equalizer(enabled, &bands);
    Ok(())
}

#[tauri::command]
pub async fn get_eq_config(
    #[allow(unused_variables)] app: AppHandle,
) -> CmdResult<tauri_plugin_ramus_ios_bridge::EqConfigResponse> {
    #[cfg(desktop)]
    {
        use ramus_core::playback::player::EQ_FREQUENCIES;
        Ok(tauri_plugin_ramus_ios_bridge::EqConfigResponse {
            frequencies: EQ_FREQUENCIES.to_vec(),
            min_gain: -12.0,
            max_gain: 12.0,
        })
    }
    #[cfg(mobile)]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .mpv_get_eq_config()
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn fetch_lyrics(
    state: State<'_, AppState>,
    rating_key: String,
) -> CmdResult<LyricsFetchResult> {
    // On-disk audio (persistent download wins over the LRU copy) lets us read a
    // cached lyrics sidecar offline, and warm one back after a live fetch.
    let audio_path = state
        .player
        .persistent_download_paths()
        .get(&rating_key)
        .cloned()
        .or_else(|| {
            state
                .player
                .with_cache(|c| c.get(&rating_key).map(|p| p.to_path_buf()))
        });

    // 1. Cached sidecar — no Plex/LRCLIB round-trip, works offline. Replaying an
    //    album the next day reads lyrics from disk instead of re-pinging LRCLIB.
    if let Some(ref audio_path) = audio_path {
        if let Some(cached) = crate::commands::downloads::read_lyrics_sidecar(audio_path).await {
            return Ok(LyricsFetchResult {
                status: lyrics::LyricsStatus::Found,
                lyrics: Some(cached),
            });
        }
    }

    // 2. Live fetch. Resolve the requested track from the queue by rating_key —
    //    LRCLIB needs its title/artist/album/duration. Falling back to
    //    `current_track` covers the rare race where mpv advanced between the UI
    //    call and this handler. With no metadata, Plex can still answer by key.
    let player_state = state.player.state();
    let track = player_state
        .queue
        .iter()
        .find(|t| t.rating_key == rating_key)
        .or(player_state.current_track.as_ref());

    // Bound the whole live fetch: the Plex client has no request timeout, so a
    // stalled-but-connected server (or a long LRCLIB retry chain) could
    // otherwise freeze the panel indefinitely. On timeout we fall through to
    // the transient path, which reports an honest offline/unreachable status.
    // Must comfortably exceed one full LRCLIB attempt (LRCLIB_TIMEOUT_SECS=15)
    // plus a quick Plex probe, or we'd kill a slow-but-successful response.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        match track {
            Some(track) => {
                lyrics::fetch_lyrics_full(
                    &state.client,
                    &state.http_client,
                    &rating_key,
                    &track.title,
                    track.display_artist(),
                    &track.album_title,
                    track.duration,
                )
                .await
            }
            // No queue metadata for LRCLIB, so only Plex can answer. It collapses
            // "no lyrics stream" and a transient failure into `None`; treat that
            // as transient so the connectivity probe picks an honest status
            // rather than claiming "not found" when we may just be offline.
            None => match lyrics::fetch_from_plex(&state.client, &rating_key).await {
                Some(found) => lyrics::LyricsOutcome::Found(found),
                None => lyrics::LyricsOutcome::Transient,
            },
        }
    })
    .await
    .unwrap_or(lyrics::LyricsOutcome::Transient);

    let result = match outcome {
        lyrics::LyricsOutcome::Found(found) => LyricsFetchResult {
            status: lyrics::LyricsStatus::Found,
            lyrics: Some(found),
        },
        lyrics::LyricsOutcome::NotFound => LyricsFetchResult {
            status: lyrics::LyricsStatus::NotFound,
            lyrics: None,
        },
        // Only on a transient failure do we probe connectivity, to tell
        // "device offline" from "lyrics source unreachable". The probe never
        // runs on the common Found/NotFound paths, so it adds no latency there.
        lyrics::LyricsOutcome::Transient => {
            let status = if crate::internet_reachable(std::time::Duration::from_secs(1)).await {
                lyrics::LyricsStatus::Unreachable
            } else {
                lyrics::LyricsStatus::Offline
            };
            LyricsFetchResult {
                status,
                lyrics: None,
            }
        }
    };

    // 3. Warm a sidecar from a live hit when the track's audio is already on
    //    disk, so a later offline replay needs no network. Best-effort and off
    //    the response path.
    if let (lyrics::LyricsStatus::Found, Some(found), Some(audio_path)) =
        (result.status, result.lyrics.as_ref(), audio_path.as_ref())
    {
        let audio_path = audio_path.clone();
        let found = found.clone();
        tauri::async_runtime::spawn(async move {
            crate::commands::downloads::write_lyrics_sidecar(&audio_path, &found).await;
        });
    }

    Ok(result)
}

#[tauri::command]
pub async fn get_waveform(
    state: State<'_, AppState>,
    rating_key: String,
) -> CmdResult<Option<Vec<f32>>> {
    // 1. Local sidecar next to the persistent download, if we have one.
    //    Populated at download time so offline playback still has the
    //    seek bar; also avoids a Plex round-trip every track change.
    if let Some(audio_path) = state
        .player
        .persistent_download_paths()
        .get(&rating_key)
        .cloned()
    {
        if let Some(levels) = crate::commands::downloads::read_waveform_sidecar(&audio_path).await {
            return Ok(Some(levels));
        }
    }

    // 2. Prefetch-cache sidecar, warmed alongside the LRU audio prefetch.
    //    Lets a replayed shuffle/album render its seek bar offline without
    //    a Plex round-trip.
    if let Some(audio_path) = state
        .player
        .with_cache(|c| c.get(&rating_key).map(|p| p.to_path_buf()))
    {
        if let Some(levels) = crate::commands::downloads::read_waveform_sidecar(&audio_path).await {
            return Ok(Some(levels));
        }
    }

    // 3. Fall back to a live Plex fetch.
    let stream = match state.client.fetch_audio_stream(&rating_key).await {
        Ok(Some(s)) => s,
        _ => return Ok(None),
    };

    let stream_id = match stream.id {
        Some(id) => id,
        None => return Ok(None),
    };

    match state.client.fetch_levels(stream_id, None).await {
        Ok(levels) if !levels.is_empty() => Ok(Some(waveform::normalize_db_levels(&levels))),
        _ => Ok(None),
    }
}

/// Push the current UI accent colour down to the OS media widget. The
/// frontend extracts the palette from album art and calls this whenever
/// the accent changes; on Android the Kotlin bridge paints the lock-screen
/// notification with the colour. Desktop + iOS accept the call and no-op.
#[tauri::command]
pub async fn set_media_accent(
    #[allow(unused_variables)] app: AppHandle,
    r: u8,
    g: u8,
    b: u8,
) -> CmdResult<()> {
    // Android: dispatch to the Kotlin plugin off the Tauri IPC thread.
    // `run_mobile_plugin` blocks until the Kotlin `@Command` resolves,
    // so we shove it onto `spawn_blocking` — this path is called during
    // normal UI work (not a Rust event-channel callback) so the strict
    // re-entrancy deadlock doesn't apply here, but the pattern is cheap
    // and matches `media_controls_android::dispatch_now_playing`.
    #[cfg(target_os = "android")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = app.ramus_ios_bridge().set_media_accent(r, g, b) {
                log::warn!("setMediaAccent failed: {e}");
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (r, g, b);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_debug_info(
    state: State<'_, AppState>,
) -> CmdResult<ramus_core::playback::player::DebugInfo> {
    Ok(state.player.debug_snapshot())
}

/// Re-sync the frontend after the OS resumes the app (unlock, foreground
/// switch, laptop wake). Stores are otherwise pure event replay, and a
/// suspended webview may have dropped every emit that fired during an
/// outage — the UI would keep showing the pre-sleep state indefinitely.
///
/// Two halves:
/// 1. Re-emit the authoritative playback + connection snapshot through the
///    normal event channels, so the existing store listeners converge
///    without any new frontend wiring.
/// 2. If the app woke up stuck offline or with playback interrupted, kick
///    a connection evaluation. This is the wake-time counterpart of the
///    stall watchdog: a suspended process runs no evals, and a
///    tunnel-shaped outage produces no path event to trigger one — so the
///    foreground transition itself is the recovery signal. Healthy-but-
///    interrupted mirrors the watchdog's kick (an unchanged-URI verdict
///    fires no monitor callback). Gated so a routine foreground while
///    everything is fine costs zero network traffic.
#[tauri::command]
pub async fn foreground_resync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let ps = state.player.state();
    emit_playback_state(
        &app,
        PlaybackStatePayload {
            status: format!("{:?}", ps.status).to_lowercase(),
            current_track: ps.current_track.clone(),
            queue_index: ps.queue_index,
        },
    );
    crate::events::emit_playback_position(
        &app,
        crate::events::PlaybackPositionPayload {
            position: state.player.position(),
            duration: state.player.duration(),
        },
    );
    let online = state
        .server_reachable
        .load(std::sync::atomic::Ordering::Acquire);
    crate::events::emit_connection_status(
        &app,
        crate::events::ConnectionStatusPayload {
            online,
            offline_mode_manual: state.settings.read().offline_mode,
            effective_offline: state.effective_offline(),
        },
    );

    // Emitted on change only, so a webview that slept through a quality step
    // would otherwise show a stale notice — or none — for the rest of the
    // session. Recomputed from live state rather than replayed, so it can't
    // resurrect a step that has since been cleared.
    crate::events::emit_playback_quality(
        &app,
        crate::events::PlaybackQualityPayload {
            starving: state.player.is_starving(),
            degraded_to_kbps: state.player.bandwidth_degrade().map(|b| b.as_kbps()),
            adaptation_blocked: !state
                .settings
                .read()
                .playback_mode
                .adapts_to_slow_connection(),
        },
    );

    if !online || state.player.needs_connection_recovery() {
        let monitor = std::sync::Arc::clone(&state.connection_monitor);
        let player = state.player.clone();
        let grace = state.recovery_grace.clone();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let outcome = monitor.evaluate_connection().await;
            // Same guard as the watchdog's: a link that is merely too slow
            // probes healthy while mpv rebuffers, and reloading there
            // discards the buffer to re-fetch it over the same short link.
            // Without this, waking the app on a starving connection fires
            // exactly the reload the watchdog now declines.
            if outcome == ramus_core::plex::connection::EvalOutcome::Healthy
                && player.is_starving()
                && crate::stall_watchdog::source_still_arriving(&player).await
            {
                log::info!("foreground resync: stream starving, not stuck; leaving mpv to buffer");
                return;
            }
            // Changed/Recovered verdicts run their own callbacks (reload,
            // online flip). Healthy fires none — the kick is ours, same as
            // the watchdog; `recover_interrupted_playback` re-checks the
            // interruption and declines for a user-paused player.
            if outcome == ramus_core::plex::connection::EvalOutcome::Healthy
                && player.recover_interrupted_playback()
            {
                log::info!("foreground resync: connection healthy, reloaded interrupted track");
                crate::events::emit_playback_buffering(&app2, true);
                // The user may re-lock immediately — keep the process
                // awake until audio flows again.
                crate::set_recovery_grace(&app2, &grace, true);
            }
        });
    }
    Ok(())
}
