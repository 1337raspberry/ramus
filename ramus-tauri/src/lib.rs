pub mod commands;
pub mod events;
pub mod state;

use std::sync::Arc;

use parking_lot::RwLock;

use ramus_core::models::Settings;
use ramus_core::playback::session::SessionTracker;
use ramus_core::plex::client::PlexClient;
use ramus_core::plex::connection::ConnectionMonitor;

use crate::state::AppState;

pub fn create_app_state() -> AppState {
    let client_identifier = uuid::Uuid::new_v4().to_string();
    let client = Arc::new(PlexClient::new(client_identifier));

    // AudioPlayer needs an MpvPlayer impl — this will be wired up
    // with real libmpv in production. For now we create a stub that
    // allows the app to compile and the commands to be tested.
    let player = Arc::new(ramus_core::playback::player::AudioPlayer::new(
        Arc::new(StubMpv),
    ));

    let connection_monitor = Arc::new(ConnectionMonitor::new(client.clone()));

    AppState {
        client,
        cache: Arc::new(parking_lot::Mutex::new(None)),
        player,
        genre_mapper: Arc::new(RwLock::new(None)),
        search_engine: Arc::new(RwLock::new(None)),
        sync_engine: Arc::new(parking_lot::Mutex::new(None)),
        session_tracker: Arc::new(parking_lot::Mutex::new(SessionTracker::default())),
        connection_monitor,
        settings: Arc::new(RwLock::new(Settings::default())),
        http_client: reqwest::Client::new(),
    }
}

/// Stub MpvPlayer for compilation. Real libmpv integration replaces this.
struct StubMpv;

impl ramus_core::playback::mpv::MpvPlayer for StubMpv {
    fn load_file(&self, _url: &str, _mode: ramus_core::playback::mpv::LoadMode) {}
    fn load_file_at(&self, _url: &str, _index: i64) {}
    fn playlist_play_index(&self, _index: i64) {}
    fn playlist_remove(&self, _index: i64) {}
    fn playlist_move(&self, _from: i64, _to: i64) {}
    fn seek(&self, _position: f64) {}
    fn set_pause(&self, _paused: bool) {}
    fn set_volume(&self, _volume: f64) {}
    fn get_volume(&self) -> f64 {
        100.0
    }
    fn set_audio_filters(&self, _value: &str) {}
    fn stop(&self) {}
    fn is_shutdown(&self) -> bool {
        false
    }
}
