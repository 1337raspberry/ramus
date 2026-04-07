use std::sync::Arc;

use tauri::State;
use url::Url;

use ramus_core::cache::db::CacheDatabase;
use ramus_core::cache::sync::SyncEngine;
use ramus_core::genre::mapper::GenreMapper;
use ramus_core::models::{LibrarySection, PlexServer};
use ramus_core::plex::auth::{self, PlexAuth};
use ramus_core::plex::token_store::{TokenKey, TokenStore};
use ramus_core::search::engine::SearchEngine;

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn start_oauth(state: State<'_, AppState>) -> CmdResult<String> {
    let auth = PlexAuth::default();
    let pin = auth
        .create_pin(&state.client.client_identifier)
        .await
        .map_err(|e| e.to_string())?;

    let url = PlexAuth::auth_url(&pin.code, &state.client.client_identifier);

    // Open the auth URL in the default browser
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
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub async fn discover_servers(state: State<'_, AppState>) -> CmdResult<Vec<PlexServer>> {
    let token = state.client.token().ok_or("Not authenticated")?;
    state
        .client
        .discover_servers(&token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_server(
    state: State<'_, AppState>,
    server: PlexServer,
) -> CmdResult<serde_json::Value> {
    let allow_http = { !state.settings.read().refuse_http };

    let (best_conn, is_http) = state.client.find_best_connection(&server, allow_http).await;
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
    server: PlexServer,
    library_key: String,
    server_url: String,
) -> CmdResult<()> {
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;

    // Store server config
    let config = ramus_core::models::ServerConfig {
        machine_identifier: server.machine_identifier.clone(),
        name: server.name.clone(),
        access_token: server.access_token.clone(),
        selected_library_key: Some(library_key.clone()),
    };
    auth::store_server_config(&config, &token_store);

    // Connect client
    let url = Url::parse(&server_url).map_err(|e| e.to_string())?;
    state
        .client
        .connect(url.clone(), server.access_token.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Initialize cache database
    let cache_dir = ramus_core::plex::token_store::config_dir()
        .map_err(|e| e.to_string())?;
    let db_path = cache_dir.join("library_cache.db");
    let db = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;
    let db_arc = Arc::new(db);

    // Set up sync engine
    let sync_engine = SyncEngine::new(db_arc.clone(), state.client.clone());
    *state.sync_engine.lock() = Some(sync_engine);

    // Set up search engine (without genre expander for now)
    let search = SearchEngine::new(db_arc.clone(), None);
    *state.search_engine.write() = Some(search);

    // Store cache reference (clone Arc, wrap inner in a new CacheDatabase isn't needed -
    // we share via the sync_engine and search_engine which hold their own Arcs)
    // For direct cache queries, open a second handle
    let db2 = CacheDatabase::open(&db_path).map_err(|e| e.to_string())?;
    *state.cache.lock() = Some(db2);

    // Configure the audio player with server connection details
    state.player.configure(
        url.clone(),
        server.access_token.clone(),
        state.client.client_identifier.clone(),
    );

    // Load genre mapper from bundled open.json
    let open_json_bytes = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json_bytes) {
        *state.genre_mapper.write() = Some(mapper);
    }

    // Start connection monitor
    state
        .connection_monitor
        .start(server, server_url, config.access_token);

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

    // Clear cache
    *state.cache.lock() = None;
    *state.sync_engine.lock() = None;
    *state.search_engine.write() = None;
    *state.genre_mapper.write() = None;

    Ok(())
}
