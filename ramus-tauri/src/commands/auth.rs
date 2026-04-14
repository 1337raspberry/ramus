use std::sync::Arc;

use tauri::State;
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
pub async fn start_oauth(state: State<'_, AppState>) -> CmdResult<String> {
    let auth = PlexAuth::default();
    let pin = auth
        .create_pin(&state.client.client_identifier)
        .await
        .map_err(|e| e.to_string())?;

    let url = PlexAuth::auth_url(&pin.code, &state.client.client_identifier);

    // Open auth URL in the default browser
    let _ = open::that(&url);

    Ok(serde_json::json!({
        "authUrl": url,
        "pinId": pin.id,
        "code": pin.code,
    })
    .to_string())
}

#[tauri::command]
pub async fn poll_oauth(
    state: State<'_, AppState>,
    pin_id: i64,
) -> CmdResult<bool> {
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
        // Terminal states — the frontend should surface these and let the
        // user restart the flow. PollingTimeout with max_attempts=1 shouldn't
        // happen in practice, but treat it as terminal to be safe.
        Err(PlexAuthError::PinExpired) => {
            Err("Sign-in code expired — please try again".into())
        }
        Err(PlexAuthError::PollingTimeout) => {
            Err("Sign-in timed out — please try again".into())
        }
        // Transient — keep polling.
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

    // Cache full server data (with tokens) server-side.
    // access_token is stripped from the response by serde skip_serializing.
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
    let (best_conn, is_http) = state.client.find_best_connection(&server, allow_http, false).await;
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

#[tauri::command]
pub async fn connect_manual_url(
    state: State<'_, AppState>,
    url: String,
) -> CmdResult<bool> {
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

    // Look up server from discovery cache. For manual connections, construct
    // a minimal server using the plex.tv auth token.
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

    // Retrieve plex.tv auth token for connection monitor re-discovery
    let auth_token = token_store
        .read(TokenKey::AuthToken)
        .unwrap_or_default();

    // Persist server config (includes connections for session restoration)
    // Validate URL scheme
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
    state
        .client
        .connect(url.clone(), server_token.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Initialize cache database and dependent subsystems
    let cache_dir = ramus_core::plex::token_store::config_dir()
        .map_err(|e| e.to_string())?;
    let db_path = cache_dir.join("library_cache.db");
    let db = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;
    let db_arc = Arc::new(db);

    let sync_engine = SyncEngine::new(db_arc.clone(), state.client.clone());
    *state.sync_engine.lock() = Some(sync_engine);

    let search = SearchEngine::new(db_arc.clone(), None);
    *state.search_engine.write() = Some(search);

    // Separate cache handle for direct queries
    let db2 = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;
    *state.cache.lock() = Some(db2);

    // Configure audio player with server connection details.
    // Normalize trailing slashes: url::Url::parse adds one but Plex connection URIs don't.
    let server_url_norm = server_url.trim_end_matches('/');
    let is_local = server.connections.iter().any(|c| {
        c.uri.trim_end_matches('/') == server_url_norm && c.local
    });
    state.player.configure(
        url.clone(),
        server_token.clone(),
        state.client.client_identifier.clone(),
    );
    state.player.set_remote(!is_local);

    // Load genre mapper from bundled data
    let open_json_bytes = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json_bytes) {
        *state.genre_mapper.write() = Some(mapper);
    }

    // Initialize connection monitor with player failover callback
    let allow_http = !state.settings.read().refuse_http;
    state.connection_monitor.set_allow_http(allow_http);
    let monitor_player = state.player.clone();
    state.connection_monitor.set_on_connection_changed(
        std::sync::Arc::new(move |url, token, is_local, _is_http| {
            let is_remote = !is_local;
            monitor_player.update_server_connection(url, token, is_remote);
            log::info!("monitor: updated player connection (is_remote={})", is_remote);
        }),
    );
    state
        .connection_monitor
        .start(PlexServer::from(&config), server_url, auth_token);

    // Clear discovery cache
    state.discovered_servers.lock().clear();

    Ok(())
}

#[tauri::command]
pub async fn is_authenticated(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.client.token().is_some())
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> CmdResult<()> {
    state.player.stop();
    state.connection_monitor.stop();
    state.client.set_token(None);
    state.client.set_server_url(None);

    // Clear stored credentials
    if let Ok(token_store) = TokenStore::new() {
        token_store.delete(TokenKey::AuthToken);
        token_store.delete(TokenKey::ServerToken);
        auth::delete_server_config(&token_store);
    }

    // Clear cache and discovery data
    *state.cache.lock() = None;
    *state.sync_engine.lock() = None;
    *state.search_engine.write() = None;
    *state.genre_mapper.write() = None;
    state.discovered_servers.lock().clear();

    Ok(())
}
