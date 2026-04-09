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
use ramus_core::models::PlexServer;
use ramus_core::plex::auth;
use ramus_core::plex::client::PlexClient;
use ramus_core::plex::connection::ConnectionMonitor;
use ramus_core::plex::token_store::{TokenKey, TokenStore};
use ramus_core::search::engine::SearchEngine;

use ramus_tauri::commands;
use ramus_tauri::state::AppState;


fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("ramus_tauri=debug,ramus_core=debug,info"),
    )
    .format_timestamp_millis()
    .init();

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Restore persistent client_identifier, or generate a new one
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
            let http_client = reqwest::Client::new();

            // Create mpv player with event callbacks
            let (player, reporter_ref) =
                ramus_tauri::create_mpv_player(app_handle, http_client.clone());

            // Create session reporter and populate the deferred callback slot
            let session_reporter =
                ramus_tauri::session_reporter::SessionReporter::new(client.clone(), player.clone());
            *reporter_ref.lock() = Some(session_reporter.clone());

            // Load saved settings and apply playback config (defaults to DirectPlay)
            let saved_settings = ramus_core::settings::load();
            player.update_config(saved_settings.to_playback_config());

            // Initialize image cache
            let image_cache_dir = ramus_core::plex::token_store::config_dir()
                .map(|d| d.join("image_cache"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/ramus_image_cache"));
            let image_cache = ramus_core::cache::image_cache::ImageCache::load(
                image_cache_dir,
                saved_settings.image_cache_limit_bytes as u64,
            );

            let state = AppState {
                client: client.clone(),
                cache: Arc::new(parking_lot::Mutex::new(None)),
                player: player.clone(),
                genre_mapper: Arc::new(RwLock::new(None)),
                search_engine: Arc::new(RwLock::new(None)),
                sync_engine: Arc::new(parking_lot::Mutex::new(None)),
                session_reporter: session_reporter.clone(),
                connection_monitor: connection_monitor.clone(),
                settings: Arc::new(RwLock::new(saved_settings)),
                image_cache: Arc::new(parking_lot::Mutex::new(image_cache)),
                http_client,
                discovered_servers: Arc::new(parking_lot::Mutex::new(Vec::new())),
            };

            // Restore previous session. State is set synchronously (no blocking
            // network call) so the window appears immediately. A background task
            // verifies connectivity and tests alternative connections.
            if let Ok(token_store) = TokenStore::new() {
                if let Some(config) = auth::stored_server_config(&token_store) {
                    if let Some(url_str) = config.active_uri.clone() {
                        if let Ok(url) = url::Url::parse(&url_str) {
                            let settings = state.settings.read().clone();

                            // Skip restoration if the stored URL is HTTP and refuse_http is enabled.
                            if settings.refuse_http && url.scheme() == "http" {
                                log::warn!(
                                    "stored server URL is HTTP but refuse_http is enabled — skipping session restore"
                                );
                            } else {
                                let token = config.access_token.clone();
                                let client_id = client.client_identifier.clone();

                                // Set client state synchronously
                                client.set_server_url(Some(url.clone()));
                                client.set_token(Some(token.clone()));

                                player.configure(url.clone(), token, client_id);

                                // Open or create cache database
                                if let Ok(cache_dir) = ramus_core::plex::token_store::config_dir() {
                                    let db_path = cache_dir.join("library_cache.db");
                                    if let Ok(db) = CacheDatabase::open(&db_path) {
                                        let db_arc = Arc::new(db);

                                        let sync = SyncEngine::new(db_arc.clone(), client.clone());
                                        *state.sync_engine.lock() = Some(sync);

                                        let search = SearchEngine::new(db_arc.clone(), None);
                                        *state.search_engine.write() = Some(search);

                                        if let Ok(db2) = CacheDatabase::open(&db_path) {
                                            *state.cache.lock() = Some(db2);
                                        }
                                    }
                                }

                                // Load genre mapper; prefer custom genres if configured
                                let custom_mapper = (settings.genre_source == ramus_core::models::GenreSource::Custom)
                                    .then(|| ramus_core::settings::load_custom_genres())
                                    .flatten()
                                    .and_then(|data| GenreMapper::from_json_bytes(&data).ok());

                                let mapper = custom_mapper.or_else(|| {
                                    let open_json = include_bytes!("../data/open.json");
                                    GenreMapper::from_json_bytes(open_json).ok()
                                });
                                if let Some(m) = mapper {
                                    *state.genre_mapper.write() = Some(m);
                                }

                                // Retrieve plex.tv auth token for monitor re-discovery
                                let auth_token = token_store
                                    .read(TokenKey::AuthToken)
                                    .unwrap_or_default();

                                // Start connection monitor
                                let allow_http = !settings.refuse_http;
                                let plex_server = PlexServer::from(&config);
                                connection_monitor.set_allow_http(allow_http);
                                let bg_auth_token = auth_token.clone();
                                connection_monitor.start(
                                    plex_server.clone(),
                                    url_str.clone(),
                                    auth_token,
                                );

                                // Background: verify current connection, test alternatives if down
                                let bg_client = client.clone();
                                let bg_url = url.clone();
                                let bg_token = config.access_token.clone();
                                let bg_monitor = connection_monitor.clone();
                                let bg_settings = state.settings.clone();
                                tauri::async_runtime::spawn(async move {
                                    log::debug!("testing stored connection: {}", bg_url);

                                    // Verify the stored URI is reachable
                                    if bg_client
                                        .test_connection(bg_url.as_str(), &bg_token, Some(std::time::Duration::from_secs(3)))
                                        .await
                                    {
                                        log::debug!("stored connection ok");
                                        return;
                                    }

                                    // Abort if logged out during the probe
                                    if bg_client.token().is_none() {
                                        return;
                                    }

                                    // Re-read in case settings changed since launch
                                    let allow_http = !bg_settings.read().refuse_http;

                                    // Stored URI is down; log cached connections for diagnosis
                                    log::info!(
                                        "stored connection unavailable ({}), testing {} cached alternative(s) (allow_http={})",
                                        bg_url,
                                        plex_server.connections.len(),
                                        allow_http,
                                    );
                                    for (i, conn) in plex_server.connections.iter().enumerate() {
                                        log::debug!(
                                            "  cached[{}]: {} (local={}, relay={}, proto={}, priority={})",
                                            i, conn.uri, conn.local, conn.relay, conn.protocol, conn.priority(),
                                        );
                                    }

                                    let (best, _is_http) = bg_client
                                        .find_best_connection(&plex_server, allow_http, true)
                                        .await;
                                    if let Some(conn) = best {
                                        if let Ok(new_url) = url::Url::parse(&conn.uri) {
                                            if bg_client.token().is_some() {
                                                bg_client.set_server_url(Some(new_url));
                                                log::info!("switched to alternative connection: {}", conn.uri);
                                                ramus_core::plex::auth::patch_stored_config(None, Some(&conn.uri));
                                                bg_monitor.update_active_uri(conn.uri);
                                            }
                                        }
                                        return;
                                    }

                                    // Cached connections exhausted — re-discover from plex.tv
                                    log::info!("no cached alternatives, re-discovering from plex.tv");
                                    if let Ok(servers) = bg_client.discover_servers(&bg_auth_token).await {
                                        if let Some(found) = servers
                                            .iter()
                                            .find(|s| s.machine_identifier == plex_server.machine_identifier)
                                        {
                                            log::debug!(
                                                "re-discovered server with {} connection(s)",
                                                found.connections.len(),
                                            );
                                            let (best, _is_http) = bg_client
                                                .find_best_connection(found, allow_http, true)
                                                .await;
                                            if let Some(conn) = best {
                                                if let Ok(new_url) = url::Url::parse(&conn.uri) {
                                                    if bg_client.token().is_some() {
                                                        bg_client.set_server_url(Some(new_url));
                                                        log::info!("switched to re-discovered connection: {}", conn.uri);
                                                        bg_monitor.update_active_uri(conn.uri.clone());
                                                        // Update monitor and disk cache
                                                        bg_monitor.update_server(found.clone());
                                                        ramus_core::plex::auth::patch_stored_config(
                                                            Some(&found.connections),
                                                            Some(&conn.uri),
                                                        );
                                                    }
                                                }
                                            } else {
                                                log::warn!(
                                                    "re-discovered server but all {} connection(s) failed",
                                                    found.connections.len(),
                                                );
                                            }
                                        } else {
                                            log::warn!(
                                                "server {} not found in {} re-discovered server(s)",
                                                plex_server.machine_identifier,
                                                servers.len(),
                                            );
                                        }
                                    } else {
                                        log::warn!("plex.tv re-discovery failed");
                                    }
                                });
                            }
                        }
                    }
                }
            }

            // Spawn auto-sync background task and start periodic session reporting
            let auto_sync_settings = state.settings.clone();
            let auto_sync_engine = state.sync_engine.clone();

            app.manage(state);

            session_reporter.ensure_loop_spawned();

            ramus_tauri::auto_sync::spawn(auto_sync_settings, auto_sync_engine);

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
            commands::library::set_album_palette,
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
            commands::settings::has_custom_genres,
            commands::settings::flush_image_cache,
            commands::settings::get_image_cache_stats,
        ])
        .build(tauri::generate_context!())
        .expect("error building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.session_reporter.stop_sync();
                }
            }
        });
}
