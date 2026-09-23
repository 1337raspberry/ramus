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
pub async fn show_native_search_bar(
    app: tauri::AppHandle,
    initial_query: String,
    top: f64,
    width: f64,
) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .show_native_search_bar(&initial_query, top, width)
            .map_err(|e| e.to_string())?;
    }
    let _ = app;
    let _ = initial_query;
    let _ = top;
    let _ = width;
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

/// Enter (`true`) or leave the full-screen visualiser's presentation: on
/// iOS the interface turns to landscape, the home indicator auto-hides
/// and the screen stays awake until it is left. A no-op elsewhere.
#[tauri::command]
pub async fn set_visualizer_presentation(app: tauri::AppHandle, active: bool) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .set_visualizer_presentation(active)
            .map_err(|e| e.to_string())?;
    }
    let _ = (app, active);
    Ok(())
}
