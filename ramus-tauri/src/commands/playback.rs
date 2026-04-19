use tauri::{AppHandle, State};

use ramus_core::models::Track;
use ramus_core::playback::lyrics::{self, LyricsResult};
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::waveform;

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
    // Report previous session stopped before loading a new queue.
    state.session_reporter.playback_stopped();

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

    Ok(())
}

#[tauri::command]
pub async fn toggle_play_pause(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.toggle_play_pause();
    Ok(())
}

#[tauri::command]
pub async fn next_track(state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.next();
    Ok(())
}

#[tauri::command]
pub async fn previous_track(state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.previous();
    Ok(())
}

#[tauri::command]
pub async fn seek(state: State<'_, AppState>, position: f64) -> CmdResult<()> {
    state.player.seek(position);
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
pub async fn append_to_queue(state: State<'_, AppState>, tracks: Vec<Track>) -> CmdResult<()> {
    state.player.append_to_queue(tracks);
    Ok(())
}

#[tauri::command]
pub async fn insert_next(state: State<'_, AppState>, tracks: Vec<Track>) -> CmdResult<()> {
    state.player.insert_next(tracks);
    Ok(())
}

#[tauri::command]
pub async fn remove_from_queue(state: State<'_, AppState>, index: usize) -> CmdResult<()> {
    state.player.remove_from_queue(index);
    Ok(())
}

#[tauri::command]
pub async fn jump_to_queue_index(state: State<'_, AppState>, index: usize) -> CmdResult<()> {
    state.prefetch_handle.notify_skip();
    state.player.jump_to_index(index);
    Ok(())
}

#[tauri::command]
pub async fn get_queue(state: State<'_, AppState>) -> CmdResult<Vec<Track>> {
    Ok(state.player.state().queue)
}

#[tauri::command]
pub async fn apply_equalizer(
    state: State<'_, AppState>,
    enabled: bool,
    bands: [f32; 10],
) -> CmdResult<()> {
    state.player.apply_equalizer(enabled, &bands);
    Ok(())
}

#[tauri::command]
pub async fn fetch_lyrics(
    state: State<'_, AppState>,
    rating_key: String,
) -> CmdResult<Option<LyricsResult>> {
    // Try Plex lyrics first; fall back to LRCLIB.
    match state.client.fetch_lyrics_stream(&rating_key).await {
        Ok(Some(stream)) => {
            if let Some(ref key) = stream.key {
                if lyrics::validate_lyrics_path(key) {
                    if let Ok(data) = state.client.download_lyrics_data(key).await {
                        if key.ends_with(".lrc") {
                            let text = String::from_utf8_lossy(&data);
                            let lines = lyrics::parse_lrc(&text);
                            if !lines.is_empty() {
                                return Ok(Some(LyricsResult {
                                    is_synced: lines.iter().any(|l| l.timestamp.is_some()),
                                    lines,
                                    source: lyrics::LyricsSource::Plex,
                                }));
                            }
                        } else {
                            if let Some(lines) = lyrics::parse_plex_json_lyrics(&data) {
                                if !lines.is_empty() {
                                    let is_synced = lines.iter().any(|l| l.timestamp.is_some());
                                    return Ok(Some(LyricsResult {
                                        lines,
                                        is_synced,
                                        source: lyrics::LyricsSource::Plex,
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(None) => {}
        Err(_) => {}
    }

    // LRCLIB fallback: look up the requested track from the queue by
    // rating_key. Falling back to `current_track` would return lyrics for
    // whatever is playing right now, which is wrong if the user opened the
    // lyrics view for a queued track or mpv advanced between the Plex
    // attempt and this call.
    let player_state = state.player.state();
    let track = player_state
        .queue
        .iter()
        .find(|t| t.rating_key == rating_key)
        .or(player_state.current_track.as_ref());
    if let Some(track) = track {
        if let Some(result) = lyrics::fetch_from_lrclib(
            &state.http_client,
            &track.title,
            track.display_artist(),
            &track.album_title,
            track.duration,
        )
        .await
        {
            return Ok(Some(result));
        }
    }

    Ok(None)
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

    // 2. Fall back to a live Plex fetch.
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
