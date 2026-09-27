//! Playback commands while a cast is active. Each maps ramus's command onto
//! the player or the server play queue, then polls at once so the UI
//! settles without waiting for the next tick. A cast that ended while a
//! command waited for the lock makes the command a no-op. Failures also go
//! out as a `cast-status` notice: transport buttons fire and forget, so the
//! toast is where a failure is seen.

use tauri::AppHandle;

use ramus_core::cast::companion::{CompanionError, PlayerCommand};
use ramus_core::cast::play_queue::{insert_chunks, UPLOAD_CHUNK};
use ramus_core::cast::timeline::RemoteState;
use ramus_core::models::Track;
use ramus_core::plex::client::build_library_uri;

use crate::state::AppState;

use super::lifecycle::{self, Source};
use super::{poll, ActiveCast, CastPlayerRef};

type OpResult = Result<(), String>;

fn player_message(player: &CastPlayerRef, error: &CompanionError) -> String {
    format!("Couldn't reach {} ({error})", player.name)
}

/// Sends a failure out as a notice (the frontend toasts it), then returns it.
fn report(app: &AppHandle, state: &AppState, result: OpResult) -> OpResult {
    if let Err(ref message) = result {
        super::emit_status(app, state, Some(message.clone()));
    }
    result
}

async fn command(app: &AppHandle, state: &AppState, command: PlayerCommand) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    let result = cast
        .client
        .send(&command)
        .await
        .map_err(|e| player_message(&cast.player, &e));
    poll::settle(app, state, &mut guard).await;
    report(app, state, result)
}

pub async fn next(app: &AppHandle, state: &AppState) -> OpResult {
    command(app, state, PlayerCommand::SkipNext).await
}

pub async fn previous(app: &AppHandle, state: &AppState) -> OpResult {
    command(app, state, PlayerCommand::SkipPrevious).await
}

pub async fn seek(app: &AppHandle, state: &AppState, position: f64) -> OpResult {
    let offset_ms = (position.max(0.0) * 1000.0) as u64;
    command(app, state, PlayerCommand::SeekTo { offset_ms }).await
}

/// Sends `source` to the player as a new play queue and mirrors it.
async fn replace_queue(
    app: &AppHandle,
    state: &AppState,
    cast: &mut ActiveCast,
    source: &Source,
) -> OpResult {
    if source.tracks.is_empty() {
        return Ok(());
    }
    let queue = lifecycle::send_queue(state, &cast.client, &cast.server, source).await?;
    lifecycle::adopt_queue(app, state, cast, &queue, source).await;
    Ok(())
}

pub async fn play_tracks(
    app: &AppHandle,
    state: &AppState,
    tracks: Vec<Track>,
    start_at: usize,
) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    let source = Source {
        tracks,
        index: start_at,
        position: 0.0,
        paused: false,
    };
    let result = replace_queue(app, state, cast, &source).await;
    poll::settle(app, state, &mut guard).await;
    report(app, state, result)
}

pub async fn toggle(app: &AppHandle, state: &AppState) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    let result = if let Some(play) = cast.state.toggle_pending() {
        // The player hasn't reported the queue just sent: flip what was sent.
        let command = if play {
            PlayerCommand::Play
        } else {
            PlayerCommand::Pause
        };
        cast.client
            .send(&command)
            .await
            .map_err(|e| player_message(&cast.player, &e))
    } else {
        match cast.state.remote_state() {
            Some(RemoteState::Playing | RemoteState::Buffering) => cast
                .client
                .send(&PlayerCommand::Pause)
                .await
                .map_err(|e| player_message(&cast.player, &e)),
            Some(RemoteState::Paused) => cast
                .client
                .send(&PlayerCommand::Play)
                .await
                .map_err(|e| player_message(&cast.player, &e)),
            // The player finished or dropped the queue: send it again from
            // the track on screen.
            _ => {
                let source = Source {
                    tracks: cast.queue.tracks(),
                    index: cast.state.index().unwrap_or(0),
                    position: 0.0,
                    paused: false,
                };
                replace_queue(app, state, cast, &source).await
            }
        }
    };
    poll::settle(app, state, &mut guard).await;
    report(app, state, result)
}

pub async fn jump(app: &AppHandle, state: &AppState, index: usize) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    let result = match cast.queue.item_at(index) {
        Some(item_id) => cast
            .client
            .send(&PlayerCommand::SkipTo { item_id })
            .await
            .map_err(|e| player_message(&cast.player, &e)),
        None => Ok(()),
    };
    poll::settle(app, state, &mut guard).await;
    report(app, state, result)
}

/// After a server queue edit: mirror the new queue, then have the player
/// reload it.
async fn after_edit(app: &AppHandle, state: &AppState, cast: &mut ActiveCast) -> OpResult {
    if poll::refetch_queue(state, cast).await {
        let effects = cast.state.remap(&cast.queue);
        poll::run_effects(app, state, effects);
        super::emit_status(app, state, None);
    }
    cast.client
        .send(&PlayerCommand::RefreshPlayQueue {
            play_queue_id: cast.queue.play_queue_id,
        })
        .await
        .map_err(|e| player_message(&cast.player, &e))
}

/// Appends `tracks`, or inserts them after the current track when `next`.
pub async fn add(app: &AppHandle, state: &AppState, tracks: Vec<Track>, next: bool) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    let keys: Vec<String> = tracks.iter().map(|t| t.rating_key.clone()).collect();
    let chunks = if next {
        insert_chunks(&keys)
    } else {
        keys.chunks(UPLOAD_CHUNK).map(<[String]>::to_vec).collect()
    };
    let mut result = Ok(());
    for chunk in chunks {
        let uri = build_library_uri(&cast.server.machine_identifier, &chunk);
        if let Err(e) = state
            .client
            .add_to_play_queue(cast.queue.play_queue_id, &uri, next)
            .await
        {
            result = Err(lifecycle::server_message(e));
            break;
        }
    }
    let refreshed = after_edit(app, state, cast).await;
    poll::settle(app, state, &mut guard).await;
    report(app, state, result.and(refreshed))
}

/// Removes the entry at `index`. The playing entry stays, as locally.
pub async fn remove(app: &AppHandle, state: &AppState, index: usize) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    if cast.state.index() == Some(index) {
        return Ok(());
    }
    let Some(item_id) = cast.queue.item_at(index) else {
        return Ok(());
    };
    let result = state
        .client
        .remove_play_queue_item(cast.queue.play_queue_id, item_id)
        .await
        .map_err(lifecycle::server_message);
    let refreshed = after_edit(app, state, cast).await;
    poll::settle(app, state, &mut guard).await;
    report(app, state, result.and(refreshed))
}

/// Moves the entry at `from` to `to`. The playing entry stays, as locally.
pub async fn move_item(app: &AppHandle, state: &AppState, from: usize, to: usize) -> OpResult {
    let mut guard = state.cast.session.lock().await;
    let Some(cast) = guard.as_mut() else {
        return Ok(());
    };
    if cast.state.index() == Some(from) {
        return Ok(());
    }
    let Some((item_id, after)) = cast.queue.move_target(from, to) else {
        return Ok(());
    };
    let result = state
        .client
        .move_play_queue_item(cast.queue.play_queue_id, item_id, after)
        .await
        .map_err(lifecycle::server_message);
    let refreshed = after_edit(app, state, cast).await;
    poll::settle(app, state, &mut guard).await;
    report(app, state, result.and(refreshed))
}

/// Re-sends everything after the webview slept (`foreground_resync`).
pub async fn resync(app: &AppHandle, state: &AppState) {
    let mut guard = state.cast.session.lock().await;
    if let Some(cast) = guard.as_mut() {
        cast.state.forget_emitted();
    }
    poll::settle(app, state, &mut guard).await;
    let still_casting = guard.is_some();
    drop(guard);
    super::emit_status(app, state, None);
    if still_casting {
        super::emit_playback(app, state);
    }
}

/// An OS media key or now-playing control, while casting.
#[derive(Debug, Clone, Copy)]
pub enum RemoteAction {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(f64),
}

pub async fn media_action(app: &AppHandle, state: &AppState, action: RemoteAction) {
    let result = match action {
        RemoteAction::Play => command(app, state, PlayerCommand::Play).await,
        RemoteAction::Pause => command(app, state, PlayerCommand::Pause).await,
        RemoteAction::Toggle => toggle(app, state).await,
        RemoteAction::Next => next(app, state).await,
        RemoteAction::Previous => previous(app, state).await,
        RemoteAction::Seek(position) => seek(app, state, position).await,
    };
    if let Err(e) = result {
        log::warn!("cast: media control failed: {e}");
    }
}
