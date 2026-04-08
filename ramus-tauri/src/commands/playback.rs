use tauri::{AppHandle, State};

use ramus_core::models::Track;
use ramus_core::playback::lyrics::{self, LyricsResult};
use ramus_core::playback::waveform;

use crate::events::{emit_playback_state, PlaybackStatePayload};
use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

fn trigger_prefetch(state: &AppState) {
    crate::prefetch::trigger(state.player.clone(), state.http_client.clone());
}

#[tauri::command]
pub async fn play_tracks(
    app: AppHandle,
    state: State<'_, AppState>,
    tracks: Vec<Track>,
    start_at: usize,
) -> CmdResult<()> {
    // Stop the previous session before loading a new queue.
    state.session_reporter.playback_stopped();

    state.player.load_queue(tracks, start_at);

    // Emit playback state so the UI updates
    let player_state = state.player.state();
    emit_playback_state(
        &app,
        PlaybackStatePayload {
            status: "playing".to_string(),
            current_track: player_state.current_track.clone(),
            queue_index: player_state.queue_index,
        },
    );

    // Start session for the new track. This is the authoritative call —
    // the mpv on_playlist_pos_change callback may not fire when the new
    // queue also starts at index 0 (playlist-pos doesn't change).
    if let Some(ref track) = player_state.current_track {
        state
            .session_reporter
            .track_started(track, &state.player.play_session_id());
    }

    trigger_prefetch(&state);

    Ok(())
}

#[tauri::command]
pub async fn toggle_play_pause(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.toggle_play_pause();
    Ok(())
}

#[tauri::command]
pub async fn next_track(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.next();
    trigger_prefetch(&state);
    Ok(())
}

#[tauri::command]
pub async fn previous_track(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.previous();
    trigger_prefetch(&state);
    Ok(())
}

#[tauri::command]
pub async fn seek(state: State<'_, AppState>, position: f64) -> CmdResult<()> {
    state.player.seek(position);
    state.session_reporter.playback_seeked(position);
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
    state: State<'_, AppState>,
    tracks: Vec<Track>,
) -> CmdResult<()> {
    state.player.append_to_queue(tracks);
    trigger_prefetch(&state);
    Ok(())
}

#[tauri::command]
pub async fn insert_next(
    state: State<'_, AppState>,
    tracks: Vec<Track>,
) -> CmdResult<()> {
    state.player.insert_next(tracks);
    trigger_prefetch(&state);
    Ok(())
}

#[tauri::command]
pub async fn remove_from_queue(
    state: State<'_, AppState>,
    index: usize,
) -> CmdResult<()> {
    state.player.remove_from_queue(index);
    Ok(())
}

#[tauri::command]
pub async fn jump_to_queue_index(
    state: State<'_, AppState>,
    index: usize,
) -> CmdResult<()> {
    state.player.jump_to_index(index);
    trigger_prefetch(&state);
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
    // Try Plex first: fetch lyrics stream info
    match state.client.fetch_lyrics_stream(&rating_key).await {
        Ok(Some(stream)) => {
            if let Some(ref key) = stream.key {
                if lyrics::validate_lyrics_path(key) {
                    if let Ok(data) = state.client.download_lyrics_data(key).await {
                        // Determine format from key extension
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
                            // Try JSON format
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

    // Fall back to LRCLIB
    let player_state = state.player.state();
    if let Some(ref track) = player_state.current_track {
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
    // Fetch audio stream (type 2) to get stream ID for levels endpoint
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
