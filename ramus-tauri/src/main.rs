#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::Arc;

use parking_lot::RwLock;
use tauri::Manager;

use ramus_core::cache::db::CacheDatabase;
use ramus_core::cache::sync::SyncEngine;
use ramus_core::genre::mapper::GenreMapper;
use ramus_core::playback::session::SessionTracker;
use ramus_core::plex::auth;
use ramus_core::plex::client::PlexClient;
use ramus_core::plex::connection::ConnectionMonitor;
use ramus_core::plex::token_store::TokenStore;
use ramus_core::search::engine::SearchEngine;

use ramus_tauri::commands;
use ramus_tauri::state::AppState;

fn load_server_url() -> Option<String> {
    let dir = ramus_core::plex::token_store::config_dir().ok()?;
    std::fs::read_to_string(dir.join("server_url.txt")).ok()
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Try to restore a persistent client_identifier, or create a new one
            let id_path = ramus_core::plex::token_store::config_dir()
                .ok()
                .map(|d| d.join("client_id.txt"));

            let client_identifier = id_path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_else(|| {
                    let id = uuid::Uuid::new_v4().to_string();
                    if let Some(ref p) = id_path {
                        let _ = std::fs::create_dir_all(p.parent().unwrap());
                        let _ = std::fs::write(p, &id);
                    }
                    id
                });

            let client = Arc::new(PlexClient::new(client_identifier));
            let connection_monitor = Arc::new(ConnectionMonitor::new(client.clone()));

            // Create real mpv player with event callbacks
            let player = ramus_tauri::create_mpv_player(app_handle);

            let state = AppState {
                client: client.clone(),
                cache: Arc::new(parking_lot::Mutex::new(None)),
                player: player.clone(),
                genre_mapper: Arc::new(RwLock::new(None)),
                search_engine: Arc::new(RwLock::new(None)),
                sync_engine: Arc::new(parking_lot::Mutex::new(None)),
                session_tracker: Arc::new(parking_lot::Mutex::new(SessionTracker::default())),
                connection_monitor: connection_monitor.clone(),
                settings: Arc::new(RwLock::new(ramus_core::settings::load())),
                http_client: reqwest::Client::new(),
            };

            // Try to restore previous session
            if let Ok(token_store) = TokenStore::new() {
                if let Some(config) = auth::stored_server_config(&token_store) {
                    // Restore auth token on the client
                    client.set_token(Some(config.access_token.clone()));

                    // Restore server connection
                    if let Some(url_str) = load_server_url() {
                        if let Ok(url) = url::Url::parse(&url_str) {
                            let token = config.access_token.clone();
                            let client_id = client.client_identifier.clone();

                            // Connect client synchronously during setup
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            let client_c = client.clone();
                            let url_c = url.clone();
                            let _ = rt.block_on(async {
                                client_c.connect(url_c, token.clone()).await
                            });

                            // Configure player
                            player.configure(url.clone(), token, client_id);

                            // Open cache database
                            if let Ok(cache_dir) = ramus_core::plex::token_store::config_dir() {
                                let db_path = cache_dir.join("library_cache.db");
                                if db_path.exists() {
                                    if let Ok(db) = CacheDatabase::open(&db_path) {
                                        let db_arc = Arc::new(db);

                                        // Sync engine
                                        let sync = SyncEngine::new(db_arc.clone(), client.clone());
                                        *state.sync_engine.lock() = Some(sync);

                                        // Search engine
                                        let search = SearchEngine::new(db_arc.clone(), None);
                                        *state.search_engine.write() = Some(search);

                                        // Cache handle for queries
                                        if let Ok(db2) = CacheDatabase::open(&db_path) {
                                            *state.cache.lock() = Some(db2);
                                        }
                                    }
                                }
                            }

                            // Load genre mapper — use custom genres if previously imported
                            let settings = state.settings.read().clone();
                            let loaded_custom = if settings.genre_source == ramus_core::models::GenreSource::Custom {
                                if let Some(data) = ramus_core::settings::load_custom_genres() {
                                    if let Ok(mapper) = GenreMapper::from_json_bytes(&data) {
                                        *state.genre_mapper.write() = Some(mapper);
                                        true
                                    } else { false }
                                } else { false }
                            } else { false };
                            if !loaded_custom {
                                let open_json = include_bytes!("../data/open.json");
                                if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
                                    *state.genre_mapper.write() = Some(mapper);
                                }
                            }

                            // Start connection monitor
                            connection_monitor.start(
                                ramus_core::models::PlexServer {
                                    machine_identifier: config.machine_identifier,
                                    name: config.name,
                                    access_token: config.access_token,
                                    owned: true,
                                    connections: vec![],
                                },
                                url_str,
                                String::new(),
                            );
                        }
                    }
                }
            }

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // auth
            commands::auth::start_oauth,
            commands::auth::poll_oauth,
            commands::auth::discover_servers,
            commands::auth::test_server,
            commands::auth::connect_manual_url,
            commands::auth::find_music_libraries,
            commands::auth::finalize_onboarding,
            commands::auth::is_authenticated,
            commands::auth::logout,
            // library
            commands::library::get_genre_tree,
            commands::library::get_albums_for_genre,
            commands::library::get_all_albums,
            commands::library::get_favourite_albums,
            commands::library::get_favourite_tracks,
            commands::library::get_albums_for_artist,
            commands::library::get_albums_for_artist_name,
            commands::library::get_albums_for_year,
            commands::library::get_tracks_for_album,
            commands::library::get_all_artists,
            commands::library::get_favourite_genre_tree,
            commands::library::toggle_album_favourite,
            commands::library::toggle_track_favourite,
            commands::library::get_album_genres,
            commands::library::get_album,
            commands::library::get_random_album,
            commands::library::get_art_url,
            commands::library::get_album_colors,
            commands::library::get_cache_stats,
            // playback
            commands::playback::play_tracks,
            commands::playback::toggle_play_pause,
            commands::playback::next_track,
            commands::playback::previous_track,
            commands::playback::seek,
            commands::playback::set_volume,
            commands::playback::get_volume,
            commands::playback::append_to_queue,
            commands::playback::insert_next,
            commands::playback::remove_from_queue,
            commands::playback::jump_to_queue_index,
            commands::playback::get_queue,
            commands::playback::apply_equalizer,
            commands::playback::fetch_lyrics,
            commands::playback::get_waveform,
            // search
            commands::search::search,
            // sync
            commands::sync::start_full_sync,
            commands::sync::start_incremental_sync,
            commands::sync::start_genre_sync,
            // settings
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::import_custom_genres,
            commands::settings::remove_custom_genres,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
