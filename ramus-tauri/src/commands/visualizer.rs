//! The native full-screen visualiser (iOS): the page asks for it when its
//! visualiser overlay opens, updates its colours and play state, and hides
//! it when the overlay unmounts. `show` fails everywhere but iOS, and on
//! iOS when the view can't draw, and the page then draws its own
//! visualiser; `update` and `hide` are no-ops when nothing is showing.

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::state::AppState;

use super::CmdResult;

/// What the plugin's `showNativeVisualizer` receives: the page's values,
/// plus the spectrum layout, which Rust owns.
#[cfg(target_os = "ios")]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowArgs {
    params: Value,
    backdrop: Value,
    playing: bool,
    layout: super::spectrum::SpectrumLayout,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    backdrop: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clear_frames: Option<bool>,
}

/// Show the native visualiser. `params` is the page's `VISUALIZER_PARAMS`,
/// `backdrop` the toned corner colours and dim.
#[tauri::command]
pub async fn show_native_visualizer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    params: Value,
    backdrop: Value,
    playing: bool,
) -> CmdResult<()> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        let args = ShowArgs {
            params,
            backdrop,
            playing,
            layout: super::spectrum::get_spectrum_layout(),
        };
        app.ramus_ios_bridge()
            .show_native_visualizer(&args)
            .map_err(|e| e.to_string())?;
        state.native_visualizer.set_active(true);
        Ok(())
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (app, state, params, backdrop, playing);
        Err("the native visualiser is iOS only".into())
    }
}

/// Update the native visualiser's colours or play state, or clear its
/// frames. A no-op while it isn't showing.
#[tauri::command]
pub async fn update_native_visualizer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    backdrop: Option<Value>,
    playing: Option<bool>,
    clear_frames: Option<bool>,
) -> CmdResult<()> {
    if !state.native_visualizer.is_active() {
        return Ok(());
    }
    let args = UpdateArgs {
        backdrop,
        playing,
        clear_frames,
    };
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .update_native_visualizer(&args)
            .map_err(|e| e.to_string())?;
    }
    let _ = (app, args);
    Ok(())
}

/// Hide the native visualiser: forwarding stops first, then the view goes.
#[tauri::command]
pub async fn hide_native_visualizer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    state.native_visualizer.set_active(false);
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        app.ramus_ios_bridge()
            .hide_native_visualizer()
            .map_err(|e| e.to_string())?;
    }
    let _ = app;
    Ok(())
}
