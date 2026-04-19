use super::CmdResult;

#[tauri::command]
pub async fn dismiss_keyboard(app: tauri::AppHandle) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .dismiss_keyboard()
            .map_err(|e| e.to_string())?;
    }
    let _ = app;
    Ok(())
}

#[tauri::command]
pub async fn show_native_search_bar(app: tauri::AppHandle, initial_query: String) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .show_native_search_bar(&initial_query)
            .map_err(|e| e.to_string())?;
    }
    let _ = app;
    let _ = initial_query;
    Ok(())
}

#[tauri::command]
pub async fn hide_native_search_bar(app: tauri::AppHandle) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .hide_native_search_bar()
            .map_err(|e| e.to_string())?;
    }
    let _ = app;
    Ok(())
}
