//! Shared mobile mpv plumbing. Both `mpv_ios.rs` and `mpv_android.rs`
//! delegate to the `tauri-plugin-ramus-ios-bridge` plugin, which fronts a
//! native `MpvBridgePlugin` (Swift on iOS, Kotlin on Android) holding the
//! real libmpv handle. This module owns the event-channel wiring so both
//! platforms convert the same JSON payloads into `MpvCallbacks` calls.

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use tauri::{ipc::Channel, AppHandle, Manager, Runtime};
use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;

use ramus_core::playback::mpv::{FileEndReason, MpvCallbacks};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PositionPayload {
    position: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DurationPayload {
    duration: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexPayload {
    index: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PausePayload {
    paused: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReasonPayload {
    reason: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct NetworkPathPayload {
    interfaces: Vec<String>,
    r#type: Option<String>,
}

/// Register one `Channel` per mpv event and wire each channel's callback
/// to the matching entry on `MpvCallbacks`. Channels live for the process
/// lifetime — the `AudioPlayer` singleton pins them.
pub fn register_mpv_listeners<R: Runtime>(
    app: &AppHandle<R>,
    callbacks: Arc<MpvCallbacks>,
) -> tauri_plugin_ramus_ios_bridge::Result<()> {
    let bridge = app.ramus_ios_bridge();

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |body| {
            if let Ok(p) = body.deserialize::<PositionPayload>() {
                if let Some(ref handler) = cb.on_position_change {
                    handler(p.position);
                }
            }
            Ok(())
        });
        bridge.register_listener("mpvPositionChange", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |body| {
            if let Ok(p) = body.deserialize::<DurationPayload>() {
                if let Some(ref handler) = cb.on_duration_change {
                    handler(p.duration);
                }
            }
            Ok(())
        });
        bridge.register_listener("mpvDurationChange", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |body| {
            if let Ok(p) = body.deserialize::<IndexPayload>() {
                if let Some(ref handler) = cb.on_playlist_pos_change {
                    handler(p.index);
                }
            }
            Ok(())
        });
        bridge.register_listener("mpvPlaylistPosChange", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |body| {
            if let Ok(p) = body.deserialize::<PausePayload>() {
                if let Some(ref handler) = cb.on_pause_change {
                    handler(p.paused);
                }
            }
            Ok(())
        });
        bridge.register_listener("mpvPauseChange", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |_body| {
            if let Some(ref handler) = cb.on_idle_active {
                handler();
            }
            Ok(())
        });
        bridge.register_listener("mpvIdleActive", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |_body| {
            if let Some(ref handler) = cb.on_file_loaded {
                handler();
            }
            Ok(())
        });
        bridge.register_listener("mpvFileLoaded", channel)?;
    }

    {
        let cb = callbacks.clone();
        let channel = Channel::new(move |body| {
            if let Ok(p) = body.deserialize::<ReasonPayload>() {
                let reason = match p.reason.as_str() {
                    "eof" => FileEndReason::Eof,
                    "stop" => FileEndReason::Stop,
                    "quit" => FileEndReason::Quit,
                    "error" => FileEndReason::Error("mpv error".to_string()),
                    "redirect" => FileEndReason::Redirect,
                    _ => FileEndReason::Unknown,
                };
                if let Some(ref handler) = cb.on_file_ended {
                    handler(reason);
                }
            }
            Ok(())
        });
        bridge.register_listener("mpvFileEnded", channel)?;
    }

    Ok(())
}

/// Register the `networkPathChange` listener that drives connection
/// failover. Called from `lib.rs::setup` AFTER `AppState` is managed —
/// without that, the listener would have no `ConnectionMonitor` to call.
///
/// On every NWPathMonitor update (Wi-Fi → cellular, hotspot drop, etc.)
/// the Swift side fires `networkPathChange` with the device's interface
/// list. We forward it to `ConnectionMonitor::handle_path_update`, which
/// debounces (500ms) and re-evaluates against the cached server
/// connections. The on-connection-changed callback then swaps the
/// player's `server_url` and rewrites stale playlist URLs — kicking us
/// off a now-unreachable LAN address before mpv has a chance to hang on
/// TCP for the full `network-timeout`.
///
/// Also seeds `ConnectionMonitor` with the current path via `getNetworkInfo`
/// — Swift's NWPathMonitor fires its first event almost immediately after
/// `init_audio()`, which runs before this listener is registered, so that
/// initial event would otherwise be lost. The Swift side caches the latest
/// snapshot in `lastPathSnapshot` for exactly this reason.
pub fn register_network_listener<R: Runtime>(
    app: &AppHandle<R>,
) -> tauri_plugin_ramus_ios_bridge::Result<()> {
    let app_for_handler = app.clone();
    let channel = Channel::new(move |body| {
        let payload = body.deserialize::<NetworkPathPayload>().unwrap_or_default();
        let interfaces: HashSet<String> = payload.interfaces.iter().cloned().collect();

        let label = payload.r#type.as_deref().unwrap_or("?");
        log::info!(
            "network path change: type={label} interfaces={:?}",
            payload.interfaces,
        );

        let Some(state) = app_for_handler.try_state::<crate::state::AppState>() else {
            return Ok(());
        };
        state.connection_monitor.handle_path_update(interfaces);
        Ok(())
    });
    app.ramus_ios_bridge()
        .register_listener("networkPathChange", channel)?;

    // Seed with the current path. `handle_path_update` no-ops on an
    // unchanged interface set, so calling this twice (here + the first
    // real NWPathMonitor event) is safe.
    if let Ok(info) = app.ramus_ios_bridge().get_network_info() {
        if !info.interfaces.is_empty() {
            let interfaces: HashSet<String> = info.interfaces.into_iter().collect();
            log::info!(
                "network monitor seeded: type={} interfaces={:?}",
                info.r#type.as_deref().unwrap_or("?"),
                interfaces,
            );
            if let Some(state) = app.try_state::<crate::state::AppState>() {
                state.connection_monitor.handle_path_update(interfaces);
            }
        }
    }

    Ok(())
}
