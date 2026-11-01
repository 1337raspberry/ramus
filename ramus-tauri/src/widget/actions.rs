//! What widget commands do, carried out through the same playback
//! commands the UI invokes, so casting, prefetch, session reporting and
//! queue persistence behave exactly as for an in-app tap.

use ramus_core::models::{AlbumFilterParams, PlaybackStatus};
use ramus_core::widget::{
    favourite_songs_queue, plan_dispatch, Dispatch, IdleAction, WidgetCommand,
};
use tauri::{AppHandle, Manager};

use super::ffi::{WIDGET_DROPPED, WIDGET_FAILED, WIDGET_OK};
use crate::commands::{library, playback};
use crate::state::AppState;

/// The playing track's rating key, whether audio is running, and the
/// track's length in seconds, from the cast when one is active.
pub fn current_playback(state: &AppState) -> (Option<String>, bool, f64) {
    if state.cast.is_active() {
        let view = state.cast.view();
        let key = view
            .index
            .and_then(|i| view.tracks.get(i))
            .map(|t| t.rating_key.clone());
        return (key, view.status == "playing", view.duration);
    }
    let ps = state.player.state();
    let duration = ps.current_track.as_ref().map_or(0.0, |t| t.duration);
    (
        ps.current_track.map(|t| t.rating_key),
        ps.status == PlaybackStatus::Playing,
        duration,
    )
}

/// Plan and run a widget command, then write a fresh snapshot whatever
/// happened, so the widget never keeps showing a state the command found
/// to be out of date.
pub async fn run_command(app: &AppHandle, cmd: WidgetCommand) -> i32 {
    // One command at a time, planning included: two taps released together
    // by the readiness gate would otherwise both see "not playing" and both
    // toggle, leaving playback paused.
    static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = RUNNING.lock().await;
    let state = app.state::<AppState>();
    let (key, playing, duration) = current_playback(&state);
    let dispatch = plan_dispatch(&cmd, key.as_deref(), playing, duration);
    let code = match run_dispatch(app, dispatch).await {
        Ok(true) => WIDGET_OK,
        Ok(false) => WIDGET_DROPPED,
        Err(e) => {
            log::warn!("widget command {dispatch:?} failed: {e}");
            WIDGET_FAILED
        }
    };
    state.widget.publish_forced().await;
    code
}

/// Carry out a planned dispatch. `Ok(false)` means there was nothing to do.
pub async fn run_dispatch(app: &AppHandle, dispatch: Dispatch) -> Result<bool, String> {
    let state = || app.state::<AppState>();
    match dispatch {
        Dispatch::Play | Dispatch::Pause => {
            playback::toggle_play_pause(app.clone(), state()).await?
        }
        Dispatch::Next => playback::next_track(app.clone(), state()).await?,
        Dispatch::Previous => playback::previous_track(app.clone(), state()).await?,
        Dispatch::SeekTo(position) => playback::seek(app.clone(), state(), position).await?,
        Dispatch::Idle(action) => return run_idle(app, action).await,
        Dispatch::Dropped => return Ok(false),
    }
    Ok(true)
}

async fn run_idle(app: &AppHandle, action: IdleAction) -> Result<bool, String> {
    let state = || app.state::<AppState>();
    let tracks = match action {
        IdleAction::FavouriteAlbum | IdleAction::UnplayedAlbum => {
            let filters = AlbumFilterParams {
                favourite_albums: action == IdleAction::FavouriteAlbum,
                unplayed: action == IdleAction::UnplayedAlbum,
                ..Default::default()
            };
            // Offline, the pick is limited to downloaded albums.
            let Some(album) =
                library::get_filtered_random_album(state(), filters, Vec::new()).await?
            else {
                return Ok(false);
            };
            library::get_tracks_for_album(state(), album.rating_key).await?
        }
        IdleAction::FavouriteSongs => {
            let favourites = library::get_favourite_tracks(state()).await?;
            let st = state();
            let downloaded = st
                .effective_offline()
                .then(|| st.player.persistent_download_paths().into_keys().collect());
            favourite_songs_queue(favourites, downloaded.as_ref(), &mut rand::thread_rng())
        }
    };
    if tracks.is_empty() {
        return Ok(false);
    }
    playback::play_tracks(app.clone(), state(), tracks, 0).await?;
    Ok(true)
}
