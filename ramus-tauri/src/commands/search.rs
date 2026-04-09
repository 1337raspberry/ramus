use tauri::State;

use ramus_core::models::SearchResult;
use ramus_core::search::parser::QueryParser;

use crate::state::AppState;

use super::CmdResult;

#[tauri::command]
pub async fn search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> CmdResult<Vec<SearchResult>> {
    let engine = state.search_engine.read();
    let engine = engine.as_ref().ok_or("Search engine not initialized")?;
    let parsed = QueryParser::parse(&query);
    engine.search(&parsed, limit.unwrap_or(20)).map_err(|e| e.to_string())
}
