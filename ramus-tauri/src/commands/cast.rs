use tauri::{AppHandle, State};

use crate::cast::{lifecycle, CastPlayerRef};
use crate::events::CastStatusPayload;
use crate::state::AppState;

use super::CmdResult;

/// The Plex players on the account that answer now.
#[tauri::command]
pub async fn list_cast_players(state: State<'_, AppState>) -> CmdResult<Vec<CastPlayerRef>> {
    lifecycle::list_players(&state).await
}

/// Move playback to a player from the last listing.
#[tauri::command]
pub async fn start_cast(
    app: AppHandle,
    state: State<'_, AppState>,
    player_id: String,
) -> CmdResult<()> {
    lifecycle::start(&app, &state, &player_id).await
}

#[tauri::command]
pub async fn get_cast_status(state: State<'_, AppState>) -> CmdResult<CastStatusPayload> {
    Ok(state.cast.status_payload(None))
}

/// Hand playback back to this device.
#[tauri::command]
pub async fn stop_cast(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    lifecycle::hand_back(&app, &state).await
}
