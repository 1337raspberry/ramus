pub mod auto_sync;
pub mod commands;
pub mod events;
pub mod media_controls;
pub mod mpv_controller;
pub mod mpv_ffi;
pub mod prefetch;
pub mod session_reporter;
pub mod spectrum_analyzer;
pub mod state;

use std::sync::Arc;

use tauri::AppHandle;

use ramus_core::playback::mpv::MpvCallbacks;

use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};

use crate::events::{
    emit_playback_position, emit_playback_state, PlaybackPositionPayload, PlaybackStatePayload,
};
use crate::media_controls::MediaControlsRef;
use crate::mpv_controller::MpvController;
use crate::mpv_ffi::MpvLib;
use crate::prefetch::PrefetchHandle;
use crate::session_reporter::ReporterRef;

/// Deferred-population slot for the prefetch control handle. `main.rs` builds
/// the empty slot and populates it after `spawn_worker()` returns; mpv callback
/// closures read the slot to pump commands (natural-advance / skip / cancel)
/// into the worker.
pub type PrefetchHandleRef = Arc<parking_lot::Mutex<Option<PrefetchHandle>>>;

/// Create an AudioPlayer backed by libmpv with Tauri event callbacks.
///
/// `prefetch_handle_ref` is a deferred slot: `main.rs` passes an empty
/// `Arc<Mutex<None>>` and populates it after `spawn_worker()` returns.
pub fn create_mpv_player(
    app_handle: AppHandle,
    prefetch_handle_ref: PrefetchHandleRef,
) -> (
    Arc<ramus_core::playback::player::AudioPlayer>,
    ReporterRef,
    MediaControlsRef,
) {
    let app1 = app_handle.clone();
    let app2 = app_handle.clone();
    let app3 = app_handle.clone();
    let app4 = app_handle.clone();
    let app7 = app_handle.clone();

    // The player is needed inside callbacks but owns the MpvController. A
    // shared Arc populated after construction breaks the cycle.
    let player_ref: Arc<parking_lot::Mutex<Option<Arc<ramus_core::playback::player::AudioPlayer>>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let pr1 = player_ref.clone();
    let pr2 = player_ref.clone();
    let pr3 = player_ref.clone();
    let pr4 = player_ref.clone();
    let pr7 = player_ref.clone();
    let pr8 = player_ref.clone();
    let pr9 = player_ref.clone();

    // Deferred session reporter; populated after player construction.
    let reporter_ref: ReporterRef = Arc::new(parking_lot::Mutex::new(None));
    let sr1 = reporter_ref.clone();
    let sr2 = reporter_ref.clone();
    let sr3 = reporter_ref.clone();

    // Deferred media controls; populated after window creation in main.rs.
    let media_controls_ref: MediaControlsRef = Arc::new(parking_lot::Mutex::new(None));
    let mc1 = media_controls_ref.clone();
    let mc2 = media_controls_ref.clone();
    let mc3 = media_controls_ref.clone();

    let ph1 = prefetch_handle_ref.clone();
    let ph2 = prefetch_handle_ref.clone();

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
                let old_dur = p.duration();
                p.handle_duration_change(dur);
                // Emit immediately so the frontend gets the new duration without
                // waiting for the next time-pos tick. Use position 0 on a track
                // boundary (previous duration 0) to avoid pairing the old track's
                // position with the new track's duration during prefetch transitions.
                let pos = if old_dur == 0.0 { 0.0 } else { p.position() };
                emit_playback_position(
                    &app2,
                    PlaybackPositionPayload {
                        position: pos,
                        duration: dur,
                    },
                );

                // Push full metadata to OS media controls once the real duration
                // is known (fires shortly after track change).
                if let Some(ref mc) = *mc1.lock() {
                    if let Some(ref track) = p.state().current_track {
                        let meta = MediaMetadata::from_track(track, pos, dur, true);
                        mc.update_metadata(&meta);
                    }
                }
            }
        })),
        on_playlist_pos_change: Some(Box::new(move |pos| {
            if let Some(ref p) = *pr3.lock() {
                // Capture previous track before state update for scrobble reporting.
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

                // Nudge the prefetch worker: a natural advance shifts the
                // lookahead window, potentially bringing a new uncached target
                // into scope. The worker starts a fresh cycle if idle, or lets
                // the running serial loop pick up the shift on its next iteration.
                if let Some(ref handle) = *ph1.lock() {
                    handle.notify_natural_advance();
                }

                // Session reporting for natural track advance only. Matching
                // rating_key means a queue reload, which play_tracks already
                // reported via track_started.
                if let Some(ref reporter) = *sr1.lock() {
                    if let Some(ref prev) = prev_track {
                        let same_track = state
                            .current_track
                            .as_ref()
                            .is_some_and(|cur| cur.rating_key == prev.rating_key);
                        if !same_track {
                            reporter.playback_stopped();
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

                if let Some(ref mc) = *mc2.lock() {
                    mc.update_playback_state(!paused, p.position());
                }
            }
        })),
        on_idle_active: Some(Box::new(move || {
            if let Some(ref p) = *pr7.lock() {
                // Scrobble the last playing track before transitioning to stopped.
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

                if let Some(ref mc) = *mc3.lock() {
                    mc.clear();
                }

                // Queue finished; stop the prefetch worker until the next queue loads.
                if let Some(ref handle) = *ph2.lock() {
                    handle.notify_cancel();
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

    // Load libmpv at runtime. `MpvLib::load()` returns a multi-line string
    // listing every path it tried; surface that verbatim if it fails.
    let mpv_lib = Arc::new(MpvLib::load().unwrap_or_else(|e| panic!("{e}")));
    let mpv = MpvController::new(mpv_lib, callbacks).expect("Failed to initialize libmpv");
    let player = Arc::new(ramus_core::playback::player::AudioPlayer::new(
        Arc::new(mpv),
    ));

    *player_ref.lock() = Some(player.clone());

    (player, reporter_ref, media_controls_ref)
}
