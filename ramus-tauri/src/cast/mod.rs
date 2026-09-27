//! The active cast: ramus driving a Plex player through the Companion
//! protocol (`ramus_core::cast`).
//!
//! While a cast is active, the local player sits stopped and its mpv events
//! are ignored. The poll task turns the player's timeline into the same
//! `playback-*` events local playback emits, so the frontend stores stay
//! pure event replay.

pub mod lifecycle;
pub mod ops;
pub(crate) mod poll;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use url::Url;

use ramus_core::cast::companion::{CompanionClient, ControllerIdentity, ServerAddress};
use ramus_core::cast::session::{CastQueue, CastState};
use ramus_core::models::Track;
use ramus_core::plex::client::PlexClient;

use crate::events::{emit_cast_status, CastStatusPayload};
use crate::state::AppState;

/// A player as the frontend sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CastPlayerRef {
    pub id: String,
    pub name: String,
    pub product: Option<String>,
}

/// A row of the player picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CastPlayerView {
    pub id: String,
    pub name: String,
    pub product: Option<String>,
    pub reachable: bool,
}

/// A player from the last listing, with the address that answered its probe.
#[derive(Debug, Clone)]
pub(crate) struct ListedPlayer {
    pub player: CastPlayerRef,
    pub uri: Option<Url>,
}

/// The cast in progress, behind `CastRuntime::session`.
pub(crate) struct ActiveCast {
    pub player: CastPlayerRef,
    pub client: CompanionClient,
    pub server: ServerAddress,
    pub queue: CastQueue,
    pub state: CastState,
}

/// The cast as last emitted, readable without the session lock.
#[derive(Debug, Clone, Default)]
pub(crate) struct CastView {
    pub player: Option<CastPlayerRef>,
    pub lost: bool,
    pub tracks: Vec<Track>,
    pub index: Option<usize>,
    pub status: String,
    pub position: f64,
    pub duration: f64,
    pub queue_revision: u64,
}

#[derive(Default)]
pub struct CastRuntime {
    active: AtomicBool,
    generation: AtomicU64,
    pub(crate) session: tokio::sync::Mutex<Option<ActiveCast>>,
    view: parking_lot::Mutex<CastView>,
    listed: parking_lot::Mutex<Vec<ListedPlayer>>,
}

impl CastRuntime {
    /// Whether playback is on another device. Read by mpv callbacks on
    /// every event, so it's a plain atomic load.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Marks a cast to `player` active and returns its generation; a poll
    /// task from an earlier cast sees the generation move and exits. Runs
    /// before the local player is stopped, so the stop's mpv events are
    /// ignored.
    pub(crate) fn activate(&self, player: CastPlayerRef) -> u64 {
        {
            let mut view = self.view.lock();
            let queue_revision = view.queue_revision;
            *view = CastView {
                player: Some(player),
                queue_revision,
                ..CastView::default()
            };
        }
        self.active.store(true, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Ends the cast: mpv events drive the UI again, and the remembered
    /// cast is forgotten. The queue revision moves so the frontend reloads
    /// the (local) queue.
    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        {
            let mut view = self.view.lock();
            let queue_revision = view.queue_revision + 1;
            *view = CastView {
                queue_revision,
                ..CastView::default()
            };
        }
        ramus_core::cast::record::clear();
    }

    /// Position and duration as last polled.
    pub(crate) fn position(&self) -> (f64, f64) {
        let view = self.view.lock();
        (view.position, view.duration)
    }

    /// The current track and position while the player plays, for the
    /// periodic position save.
    pub(crate) fn playing_position(&self) -> Option<(String, f64)> {
        let view = self.view.lock();
        if view.status != "playing" {
            return None;
        }
        let track = view.index.and_then(|i| view.tracks.get(i))?;
        Some((track.rating_key.clone(), view.position))
    }

    pub(crate) fn view(&self) -> CastView {
        self.view.lock().clone()
    }

    pub(crate) fn update_view(&self, f: impl FnOnce(&mut CastView)) {
        f(&mut self.view.lock());
    }

    pub(crate) fn set_listed(&self, players: Vec<ListedPlayer>) {
        *self.listed.lock() = players;
    }

    pub(crate) fn listed(&self, id: &str) -> Option<ListedPlayer> {
        self.listed
            .lock()
            .iter()
            .find(|p| p.player.id == id)
            .cloned()
    }

    pub fn status_payload(&self, notice: Option<String>) -> CastStatusPayload {
        let view = self.view.lock();
        CastStatusPayload {
            player: view.player.clone(),
            link: view
                .player
                .as_ref()
                .map(|_| if view.lost { "lost" } else { "connected" }),
            queue_revision: view.queue_revision,
            notice,
        }
    }
}

/// For mpv callbacks, which hold an `AppHandle` rather than the state.
pub(crate) fn is_casting<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<AppState>()
        .is_some_and(|s| s.cast.is_active())
}

pub(crate) fn identity(client: &PlexClient) -> ControllerIdentity {
    ControllerIdentity {
        client_identifier: client.client_identifier.clone(),
        platform: PlexClient::platform(),
        device: PlexClient::device(),
    }
}

pub(crate) fn emit_status(app: &AppHandle, state: &AppState, notice: Option<String>) {
    emit_cast_status(app, state.cast.status_payload(notice));
}

/// Re-sends the cast's playback state and position from the view.
pub(crate) fn emit_playback(app: &AppHandle, state: &AppState) {
    let view = state.cast.view();
    let status = if view.status.is_empty() {
        "stopped".to_string()
    } else {
        view.status.clone()
    };
    crate::events::emit_playback_state(
        app,
        crate::events::PlaybackStatePayload {
            status,
            current_track: view.index.and_then(|i| view.tracks.get(i).cloned()),
            queue_index: view.index.unwrap_or(0),
        },
    );
    crate::events::emit_playback_position(
        app,
        crate::events::PlaybackPositionPayload {
            position: view.position,
            duration: view.duration,
        },
    );
}
