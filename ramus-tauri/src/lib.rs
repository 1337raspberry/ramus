pub mod auto_sync;
pub mod commands;
pub mod events;
pub mod mpv_controller;
pub mod mpv_ffi;
pub mod prefetch;
pub mod session_reporter;
pub mod spectrum_analyzer;
pub mod state;

use std::sync::Arc;

use tauri::AppHandle;

use ramus_core::playback::mpv::MpvCallbacks;

use crate::events::{
    emit_playback_buffering, emit_playback_position, emit_playback_state,
    PlaybackBufferingPayload, PlaybackPositionPayload, PlaybackStatePayload,
};
use crate::mpv_controller::MpvController;
use crate::mpv_ffi::MpvLib;
use crate::session_reporter::ReporterRef;

/// Create an AudioPlayer backed by libmpv with Tauri event callbacks.
/// Returns the player and a deferred `SessionReporter` slot that must
/// be populated after construction.
pub fn create_mpv_player(
    app_handle: AppHandle,
    http_client: reqwest::Client,
) -> (Arc<ramus_core::playback::player::AudioPlayer>, ReporterRef) {
    let app1 = app_handle.clone();
    let app3 = app_handle.clone();
    let app4 = app_handle.clone();
    let app5 = app_handle.clone();
    let app6 = app_handle.clone();
    let app7 = app_handle.clone();

    // The player is needed inside callbacks but holds the MpvController.
    // Use a shared Arc populated after construction to break the cycle.
    let player_ref: Arc<parking_lot::Mutex<Option<Arc<ramus_core::playback::player::AudioPlayer>>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let pr1 = player_ref.clone();
    let pr2 = player_ref.clone();
    let pr3 = player_ref.clone();
    let pr4 = player_ref.clone();
    let pr5 = player_ref.clone();
    let pr6 = player_ref.clone();
    let pr7 = player_ref.clone();
    let pr8 = player_ref.clone();
    let pr9 = player_ref.clone();

    // Deferred session reporter, populated after player construction.
    let reporter_ref: ReporterRef = Arc::new(parking_lot::Mutex::new(None));
    let sr1 = reporter_ref.clone();
    let sr2 = reporter_ref.clone();
    let sr3 = reporter_ref.clone();

    let callbacks = Arc::new(MpvCallbacks {
        on_position_change: Some(Box::new(move |pos| {
            if let Some(ref p) = *pr1.lock() {
                p.handle_position_change(pos);
                let dur = p.duration();
                emit_playback_position(
                    &app1,
                    PlaybackPositionPayload {
                        position: pos,
                        duration: dur,
                    },
                );
            }
        })),
        on_duration_change: Some(Box::new(move |dur| {
            if let Some(ref p) = *pr2.lock() {
                p.handle_duration_change(dur);
            }
        })),
        on_playlist_pos_change: Some(Box::new(move |pos| {
            if let Some(ref p) = *pr3.lock() {
                // Capture previous track before state update for scrobble reporting
                let prev_track = p.state().current_track.clone();

                p.handle_playlist_pos_change(pos);
                let state = p.state();
                emit_playback_state(
                    &app3,
                    PlaybackStatePayload {
                        status: format!("{:?}", state.status).to_lowercase(),
                        current_track: state.current_track.clone(),
                        queue_index: state.queue_index,
                    },
                );
                            prefetch::trigger(p.clone(), http_client.clone(), app3.clone());

                // Session reporting for natural track advance only.
                // Same rating_key as previous track indicates a queue reload;
                // play_tracks already handled track_started in that case.
                if let Some(ref reporter) = *sr1.lock() {
                    if let Some(ref prev) = prev_track {
                        let same_track = state
                            .current_track
                            .as_ref()
                            .is_some_and(|cur| cur.rating_key == prev.rating_key);
                        if !same_track {
                            reporter.track_ended(prev);
                            if let Some(ref track) = state.current_track {
                                reporter.track_started(track, &p.play_session_id());
                            }
                        }
                    }
                }
            }
        })),
        on_pause_change: Some(Box::new(move |paused| {
            if let Some(ref p) = *pr4.lock() {
                p.handle_pause_change(paused);
                let state = p.state();
                emit_playback_state(
                    &app4,
                    PlaybackStatePayload {
                        status: if paused {
                            "paused".to_string()
                        } else {
                            "playing".to_string()
                        },
                        current_track: state.current_track,
                        queue_index: state.queue_index,
                    },
                );

                if let Some(ref reporter) = *sr2.lock() {
                    if paused {
                        reporter.playback_paused();
                    } else {
                        reporter.playback_resumed();
                    }
                }
            }
        })),
        on_buffering_change: Some(Box::new(move |buffering| {
            if let Some(ref p) = *pr5.lock() {
                p.handle_buffering_change(buffering);
                let snap = p.snapshot();
                emit_playback_buffering(
                    &app5,
                    PlaybackBufferingPayload {
                        is_buffering: buffering,
                        buffered_fraction: snap.buffered_fraction,
                    },
                );
            }
        })),
        on_cache_state_change: Some(Box::new(move |state| {
            if let Some(ref p) = *pr6.lock() {
                p.handle_cache_state_change(state);
                emit_playback_buffering(
                    &app6,
                    PlaybackBufferingPayload {
                        is_buffering: false,
                        buffered_fraction: (state as f64 / 100.0).clamp(0.0, 1.0),
                    },
                );
            }
        })),
        on_idle_active: Some(Box::new(move || {
            if let Some(ref p) = *pr7.lock() {
                // Scrobble the last playing track before transitioning to stopped
                if let Some(ref reporter) = *sr3.lock() {
                    if let Some(ref track) = p.state().current_track {
                        reporter.track_ended(track);
                    }
                }

                p.handle_idle_active();
                emit_playback_state(
                    &app7,
                    PlaybackStatePayload {
                        status: "stopped".to_string(),
                        current_track: None,
                        queue_index: 0,
                    },
                );

                if let Some(ref reporter) = *sr3.lock() {
                    reporter.playback_stopped();
                }
            }
        })),
        on_file_loaded: Some(Box::new(move || {
            if let Some(ref p) = *pr8.lock() {
                p.handle_file_loaded();
            }
        })),
        on_file_ended: Some(Box::new(move |reason| {
            if let Some(ref p) = *pr9.lock() {
                p.handle_file_ended(reason);
            }
        })),
    });

    // Load libmpv at runtime. A clear error beats an obscure dlopen failure,
    // so `MpvLib::load()` already returns a multi-line string listing every
    // path it tried — surface that verbatim if it fails.
    let mpv_lib = Arc::new(MpvLib::load().unwrap_or_else(|e| panic!("{e}")));
    let mpv = MpvController::new(mpv_lib, callbacks).expect("Failed to initialize libmpv");
    let player = Arc::new(ramus_core::playback::player::AudioPlayer::new(
        Arc::new(mpv),
    ));

    // Populate the deferred player reference for callbacks
    *player_ref.lock() = Some(player.clone());

    (player, reporter_ref)
}

