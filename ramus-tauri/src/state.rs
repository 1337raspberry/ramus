use std::sync::Arc;

use parking_lot::RwLock;

use ramus_core::cache::db::CacheDatabase;
use ramus_core::cache::sync::SyncEngine;
use ramus_core::genre::mapper::GenreMapper;
use ramus_core::models::Settings;
use ramus_core::playback::player::AudioPlayer;
use ramus_core::playback::session::SessionTracker;
use ramus_core::plex::client::PlexClient;
use ramus_core::plex::connection::ConnectionMonitor;
use ramus_core::search::engine::SearchEngine;

pub struct AppState {
    pub client: Arc<PlexClient>,
    pub cache: Arc<parking_lot::Mutex<Option<CacheDatabase>>>,
    pub player: Arc<AudioPlayer>,
    pub genre_mapper: Arc<RwLock<Option<GenreMapper>>>,
    pub search_engine: Arc<RwLock<Option<SearchEngine>>>,
    pub sync_engine: Arc<parking_lot::Mutex<Option<SyncEngine>>>,
    pub session_tracker: Arc<parking_lot::Mutex<SessionTracker>>,
    pub connection_monitor: Arc<ConnectionMonitor>,
    pub settings: Arc<RwLock<Settings>>,
    pub http_client: reqwest::Client,
}
