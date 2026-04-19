use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use ramus_core::cache::db::CacheDatabase;
use ramus_core::cache::image_cache::ImageCache;
use ramus_core::cache::sync::SyncEngine;
use ramus_core::genre::mapper::GenreMapper;
use ramus_core::models::{PlexServer, Settings};
use ramus_core::playback::player::AudioPlayer;
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
    pub session_reporter: Arc<crate::session_reporter::SessionReporter>,
    pub connection_monitor: Arc<ConnectionMonitor>,
    pub settings: Arc<RwLock<Settings>>,
    pub image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    pub http_client: reqwest::Client,
    /// Control surface for the background prefetch worker. Cheap to clone —
    /// an mpsc sender plus an atomic generation counter.
    pub prefetch_handle: crate::prefetch::PrefetchHandle,
    /// Servers from the last `discover_servers` call, keyed by machine_identifier.
    /// Holds full data including tokens so the frontend never needs them.
    pub discovered_servers: Arc<parking_lot::Mutex<Vec<PlexServer>>>,
    /// OS-level media controls (Now Playing, media keys). `None` if platform
    /// init failed (e.g. no D-Bus on Linux).
    pub media_controls: crate::media_controls::MediaControlsRef,
    /// Prevents overlapping sync operations from corrupting the database.
    pub sync_in_progress: Arc<AtomicBool>,
    /// Live server reachability as reported by `ConnectionMonitor`. Starts
    /// `true` on boot; flipped by the startup probe and on-connection-lost
    /// callbacks. Read together with `Settings.offline_mode` to determine
    /// whether library queries should filter to downloaded-only content.
    pub server_reachable: Arc<AtomicBool>,
}

impl AppState {
    /// Whether the app should present an offline UI — either the user
    /// ticked "Work Offline" in settings or the server is unreachable.
    pub fn effective_offline(&self) -> bool {
        self.settings.read().offline_mode || !self.server_reachable.load(Ordering::Acquire)
    }
}
