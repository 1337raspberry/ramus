use tauri::State;

use ramus_core::models::{Album, SearchResponse};
use ramus_core::search::parser::QueryParser;

use crate::state::AppState;

use super::CmdResult;

/// Sectioned search: artists / albums / tracks / genres, ordered by
/// relevance. `section_limit` caps each section independently (the UI
/// shows a few rows per section and expands on demand).
#[tauri::command]
pub async fn search(
    state: State<'_, AppState>,
    query: String,
    section_limit: Option<usize>,
) -> CmdResult<SearchResponse> {
    let engine = state.search_engine.read();
    let engine = engine.as_ref().ok_or("Search engine not initialized")?;
    let parsed = QueryParser::parse(&query);
    engine
        .search_sectioned(&parsed, section_limit.unwrap_or(20))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_albums_for_grid(
    state: State<'_, AppState>,
    query: String,
) -> CmdResult<Vec<Album>> {
    let parsed = QueryParser::parse(&query);
    let ids = {
        let engine = state.search_engine.read();
        let engine = engine.as_ref().ok_or("Search engine not initialized")?;
        engine
            .search_album_ids(&parsed)
            .map_err(|e| e.to_string())?
    };
    let cache = state.cache.lock();
    let db = cache.as_ref().ok_or("Cache not initialized")?;
    db.albums_by_internal_ids(&ids).map_err(|e| e.to_string())
}
