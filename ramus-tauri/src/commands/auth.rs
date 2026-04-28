use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use url::Url;

use ramus_core::cache::db::CacheDatabase;
use ramus_core::cache::sync::SyncEngine;
use ramus_core::genre::mapper::GenreMapper;
use ramus_core::models::{LibrarySection, PlexServer};
use ramus_core::plex::auth::{self, PlexAuth, PlexAuthError};
use ramus_core::plex::token_store::{TokenKey, TokenStore};
use ramus_core::search::engine::SearchEngine;

use crate::state::AppState;

use super::CmdResult;

#[tauri::command]
pub async fn start_oauth(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let auth = PlexAuth::default();
    let pin = auth
        .create_pin(&state.client.client_identifier)
        .await
        .map_err(|e| e.to_string())?;

    let url = PlexAuth::auth_url(&pin.code, &state.client.client_identifier);

    // `tauri-plugin-opener` dispatches to the right platform backend:
    // `UIApplication.open` on iOS, `NSWorkspace.open` on macOS,
    // `ShellExecuteW` on Windows, `xdg-open` on Linux. The old `open`
    // crate no-opped on iOS, which broke the OAuth flow in the
    // simulator.
    if let Err(e) = app.opener().open_url(&url, None::<&str>) {
        log::warn!("failed to open auth URL in external browser: {e}");
    }

    Ok(serde_json::json!({
        "authUrl": url,
        "pinId": pin.id,
    })
    .to_string())
}

#[tauri::command]
pub async fn poll_oauth(state: State<'_, AppState>, pin_id: i64) -> CmdResult<bool> {
    let auth = PlexAuth::default();
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;

    match auth
        .poll_for_token(
            pin_id,
            &state.client.client_identifier,
            &token_store,
            1,
            std::time::Duration::from_secs(0),
        )
        .await
    {
        Ok(token) => {
            state.client.set_token(Some(token));
            Ok(true)
        }
        // Terminal: the frontend surfaces this and restarts the flow.
        Err(PlexAuthError::PinExpired) => Err("Sign-in code expired — please try again".into()),
        // Transient: no token yet (PollingTimeout here just means the single
        // attempt didn't find a token — the frontend drives the polling
        // cadence), or a network blip. Keep polling.
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub async fn discover_servers(state: State<'_, AppState>) -> CmdResult<Vec<PlexServer>> {
    let token = state.client.token().ok_or("Not authenticated")?;
    let servers = state
        .client
        .discover_servers(&token)
        .await
        .map_err(|e| e.to_string())?;

    // Cache full server data (with tokens) server-side; access_token is
    // stripped from the frontend response via serde skip_serializing.
    *state.discovered_servers.lock() = servers.clone();

    Ok(servers)
}

#[tauri::command]
pub async fn test_server(
    state: State<'_, AppState>,
    machine_identifier: String,
) -> CmdResult<serde_json::Value> {
    let server = state
        .discovered_servers
        .lock()
        .iter()
        .find(|s| s.machine_identifier == machine_identifier)
        .cloned()
        .ok_or("Server not found — run discover first")?;

    let allow_http = !state.settings.read().refuse_http;
    let (best_conn, is_http) = state
        .client
        .find_best_connection(&server, allow_http, false)
        .await;
    match best_conn {
        Some(conn) => Ok(serde_json::json!({
            "connected": true,
            "uri": conn.uri,
            "local": conn.local,
            "isHttp": is_http,
        })),
        None => Ok(serde_json::json!({ "connected": false })),
    }
}

/// Connect the live client to a server returned by `discover_servers`,
/// using that server's per-account access token (the master plex.tv token
/// is rejected by shared servers). The frontend calls this after the user
/// picks a server and before `find_music_libraries`, so library listing
/// hits the right URL with a token Plex will accept.
#[tauri::command]
pub async fn connect_to_discovered(
    state: State<'_, AppState>,
    machine_identifier: String,
) -> CmdResult<serde_json::Value> {
    let server = state
        .discovered_servers
        .lock()
        .iter()
        .find(|s| s.machine_identifier == machine_identifier)
        .cloned()
        .ok_or("Server not found — run discover first")?;

    let allow_http = !state.settings.read().refuse_http;
    // `send_token = false`: probing with the token would notify the server
    // owner once per candidate. `connect()` below sends it exactly once.
    let (best_conn, is_http) = state
        .client
        .find_best_connection(&server, allow_http, false)
        .await;
    let conn = best_conn.ok_or("No reachable connection for server")?;
    let url = Url::parse(&conn.uri).map_err(|e| e.to_string())?;

    state
        .client
        .connect(url, server.access_token.clone())
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "uri": conn.uri,
        "local": conn.local,
        "isHttp": is_http,
    }))
}

#[tauri::command]
pub async fn connect_manual_url(state: State<'_, AppState>, url: String) -> CmdResult<bool> {
    let token = state.client.token().ok_or("Not authenticated")?;
    let parsed = Url::parse(&url).map_err(|e| e.to_string())?;

    match parsed.scheme() {
        "https" => {}
        "http" => {
            if state.settings.read().refuse_http {
                return Err("HTTP connections are disabled in settings".into());
            }
        }
        other => return Err(format!("Unsupported scheme: {other}")),
    }

    state
        .client
        .connect(parsed, token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn find_music_libraries(state: State<'_, AppState>) -> CmdResult<Vec<LibrarySection>> {
    state
        .client
        .find_music_libraries()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn finalize_onboarding(
    state: State<'_, AppState>,
    machine_identifier: String,
    library_key: String,
    server_url: String,
) -> CmdResult<()> {
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;

    // Look up server from discovery cache; manual connections get a minimal
    // server using the plex.tv auth token.
    let server = state
        .discovered_servers
        .lock()
        .iter()
        .find(|s| s.machine_identifier == machine_identifier)
        .cloned()
        .unwrap_or_else(|| PlexServer {
            machine_identifier: machine_identifier.clone(),
            name: server_url.clone(),
            access_token: String::new(),
            owned: true,
            connections: vec![],
        });

    let server_token = if server.access_token.is_empty() {
        state.client.token().ok_or("Not authenticated")?
    } else {
        server.access_token.clone()
    };

    // plex.tv auth token for connection monitor re-discovery.
    let auth_token = token_store.read(TokenKey::AuthToken).unwrap_or_default();

    let url = Url::parse(&server_url).map_err(|e| e.to_string())?;
    match url.scheme() {
        "https" => {}
        "http" => {
            if state.settings.read().refuse_http {
                return Err("HTTP connections are disabled in settings".into());
            }
        }
        other => return Err(format!("Unsupported scheme: {other}")),
    }

    let config = ramus_core::models::ServerConfig {
        machine_identifier: server.machine_identifier.clone(),
        name: server.name.clone(),
        access_token: server_token.clone(),
        selected_library_key: Some(library_key.clone()),
        owned: server.owned,
        connections: server.connections.clone(),
        active_uri: Some(server_url.clone()),
    };
    if !auth::store_server_config(&config, &token_store) {
        return Err("Failed to persist server credentials".into());
    }

    // Open both DB handles before touching state.client so a failure during
    // cache init doesn't leave the client carrying a token while
    // state.cache / sync_engine / search_engine are still empty —
    // is_authenticated would otherwise return true on a half-init state.
    let cache_dir = ramus_core::plex::token_store::config_dir().map_err(|e| e.to_string())?;
    let db_path = cache_dir.join("library_cache.db");
    let db = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;
    let db_arc = Arc::new(db);
    let db2 = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;

    state
        .client
        .connect(url.clone(), server_token.clone())
        .await
        .map_err(|e| e.to_string())?;

    let sync_engine = SyncEngine::new(db_arc.clone(), state.client.clone());
    *state.sync_engine.lock() = Some(sync_engine);

    let search = SearchEngine::new(db_arc.clone(), None);
    *state.search_engine.write() = Some(search);

    crate::prefetch::rehydrate_persistent_downloads(&state.player, &db2);
    *state.cache.lock() = Some(db2);
    crate::commands::downloads::recompute_image_pins(&state);

    // url::Url::parse adds a trailing slash that Plex connection URIs don't have.
    let server_url_norm = server_url.trim_end_matches('/');
    let is_local = server
        .connections
        .iter()
        .any(|c| c.uri.trim_end_matches('/') == server_url_norm && c.local);
    state.player.configure(
        url.clone(),
        server_token.clone(),
        state.client.client_identifier.clone(),
    );
    state.player.set_remote(!is_local);

    let open_json_bytes = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json_bytes) {
        mapper.set_threshold(state.settings.read().genre_fuzzy_threshold);
        *state.genre_mapper.write() = Some(mapper);
    }

    // Connection monitor with player failover callback.
    let allow_http = !state.settings.read().refuse_http;
    state.connection_monitor.set_allow_http(allow_http);
    let monitor_player = state.player.clone();
    let monitor_prefetch = state.prefetch_handle.clone();
    state
        .connection_monitor
        .set_on_connection_changed(std::sync::Arc::new(
            move |url, token, is_local, _is_http| {
                let is_remote = !is_local;
                monitor_player.update_server_connection(url, token, is_remote);
                monitor_player.rewrite_stale_playlist_urls();
                monitor_prefetch.notify_skip();
                log::info!(
                    "monitor: updated player connection (is_remote={})",
                    is_remote
                );
            },
        ));
    state
        .connection_monitor
        .start(PlexServer::from(&config), server_url, auth_token);

    state.discovered_servers.lock().clear();

    Ok(())
}

#[tauri::command]
pub async fn is_authenticated(state: State<'_, AppState>) -> CmdResult<bool> {
    // A token alone is not enough — after OAuth but before the user picks
    // a server + library, `poll_oauth` has set the token but no server
    // config exists yet. On iOS this matters because the WKWebView
    // reloads its JS state when the user returns from Safari after
    // completing the PIN flow; App.tsx re-mounts, calls this command,
    // and without the server-url check it would skip straight past the
    // onboarding's server/library pickers into a broken main UI.
    Ok(state.client.token().is_some() && state.client.server_url().is_some())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.stop();
    state.connection_monitor.stop();
    state.client.set_token(None);
    state.client.set_server_url(None);

    match TokenStore::new() {
        Ok(token_store) => {
            token_store.delete(TokenKey::AuthToken);
            token_store.delete(TokenKey::ServerToken);
            auth::delete_server_config(&token_store);
        }
        Err(e) => {
            log::error!("logout: token store unavailable, persisted tokens not deleted: {e}");
        }
    }

    *state.cache.lock() = None;
    *state.sync_engine.lock() = None;
    *state.search_engine.write() = None;
    *state.genre_mapper.write() = None;
    state.discovered_servers.lock().clear();

    Ok(())
}
