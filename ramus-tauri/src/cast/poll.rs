//! The poll loop: one timeline poll per tick, turned into playback events
//! by the `CastState` reducer.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use ramus_core::cast::play_queue::PlayQueue;
use ramus_core::cast::session::{poll_delay, CastEffect, CastQueue, Link};
#[cfg(desktop)]
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};

use crate::events::{
    emit_playback_buffering, emit_playback_position, emit_playback_state, webview_hidden,
    PlaybackPositionPayload, PlaybackStatePayload,
};
use crate::state::AppState;

use super::{lifecycle, ActiveCast};

pub(crate) type SessionGuard<'a> = tokio::sync::MutexGuard<'a, Option<ActiveCast>>;

/// Polls the player until the cast with `generation` ends.
pub(crate) fn spawn(app: AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let mut delay = Duration::from_secs(1);
        loop {
            tokio::time::sleep(delay).await;
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let mut guard = state.cast.session.lock().await;
            if state.cast.generation() != generation {
                break;
            }
            settle(&app, &state, &mut guard).await;
            let Some(cast) = guard.as_ref() else { break };
            delay = poll_delay(
                cast.state.link(),
                cast.state.failures(),
                webview_hidden(&app),
            );
        }
    });
}

/// Polls once and applies the result. A takeover ends the cast here.
pub(crate) async fn settle(app: &AppHandle, state: &AppState, guard: &mut SessionGuard<'_>) {
    let Some(cast) = guard.as_mut() else { return };
    let outcome = cast.client.poll().await;
    let mut effects = cast.state.apply(&outcome, &cast.queue);
    if effects.contains(&CastEffect::RefetchQueue) && refetch_queue(state, cast).await {
        effects.extend(cast.state.remap(&cast.queue));
        super::emit_status(app, state, None);
    }
    if let Some(ending) = run_effects(app, state, effects) {
        if let Some(cast) = guard.take() {
            lifecycle::finish_takeover(app, state, cast, ending);
        }
    }
}

/// Refetches the whole play queue into the mirror. False when the server
/// didn't answer; the next poll asks again.
pub(crate) async fn refetch_queue(state: &AppState, cast: &mut ActiveCast) -> bool {
    match state.client.play_queue(cast.queue.play_queue_id).await {
        Ok(queue) => {
            set_queue(state, cast, &queue);
            true
        }
        Err(e) => {
            log::warn!("cast: play queue refetch failed: {e}");
            false
        }
    }
}

/// Installs `queue` as the mirror, resolving its tracks against the library.
pub(crate) fn set_queue(state: &AppState, cast: &mut ActiveCast, queue: &PlayQueue) {
    cast.queue = {
        let cache = state.cache.lock();
        CastQueue::resolve(queue, |rating_key| {
            cache
                .as_ref()
                .and_then(|db| db.track_by_source_id(rating_key).ok().flatten())
        })
    };
    let tracks = cast.queue.tracks();
    state.cast.update_view(|v| {
        v.tracks = tracks;
        v.queue_revision += 1;
    });
}

/// How a cast ended on the player's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    TakenOver,
    NotStarted,
}

/// Performs the reducer's effects. Returns how the cast ended, when the
/// player dropped it.
pub(crate) fn run_effects(
    app: &AppHandle,
    state: &AppState,
    effects: Vec<CastEffect>,
) -> Option<Ending> {
    let mut ending = None;
    let mut state_changed = false;
    for effect in effects {
        match effect {
            CastEffect::EmitState { status, index } => {
                state.cast.update_view(|v| {
                    v.status = status.to_string();
                    v.index = index;
                });
                state_changed = true;
            }
            CastEffect::EmitPosition { position, duration } => {
                state.cast.update_view(|v| {
                    v.position = position;
                    v.duration = duration;
                });
                emit_playback_position(app, PlaybackPositionPayload { position, duration });
            }
            CastEffect::EmitBuffering(buffering) => emit_playback_buffering(app, buffering),
            CastEffect::MarkPlayed(rating_key) => mark_played(state, &rating_key),
            CastEffect::Link(link) => {
                state.cast.update_view(|v| v.lost = link == Link::Lost);
                super::emit_status(app, state, None);
            }
            CastEffect::TakenOver => ending = Some(Ending::TakenOver),
            CastEffect::NotStarted => ending = Some(Ending::NotStarted),
            CastEffect::RefetchQueue => {}
        }
    }
    if state_changed && ending.is_none() {
        let view = state.cast.view();
        let track = view.index.and_then(|i| view.tracks.get(i).cloned());
        let status = if view.status.is_empty() {
            "stopped".to_string()
        } else {
            view.status
        };
        #[cfg(desktop)]
        {
            if let Some(ref mc) = *state.media_controls.lock() {
                match track {
                    Some(ref t) => mc.update_metadata(&MediaMetadata::from_track(
                        t,
                        view.position,
                        view.duration,
                        status == "playing",
                    )),
                    None => mc.clear(),
                }
            }
        }
        emit_playback_state(
            app,
            PlaybackStatePayload {
                status,
                current_track: track,
                queue_index: view.index.unwrap_or(0),
            },
        );
        crate::queue_persist::save_soon(app);
    }
    ending
}

/// Records a play in the local library, as local playback does at 90 %.
/// The player reports the play to the server itself.
fn mark_played(state: &AppState, rating_key: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if let Some(db) = state.cache.lock().as_ref() {
        if let Err(e) = db.mark_track_played(rating_key, now) {
            log::warn!("cast: could not record local play state: {e}");
        }
    }
}
