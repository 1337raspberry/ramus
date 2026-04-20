//! Shared mobile mpv plumbing. Both `mpv_ios.rs` and `mpv_android.rs`
//! delegate to the `tauri-plugin-ramus-ios-bridge` plugin, which fronts a
//! native `MpvBridgePlugin` (Swift on iOS, Kotlin on Android) holding the
//! real libmpv handle. This module owns the event-channel wiring so both
//! platforms convert the same JSON payloads into `MpvCallbacks` calls.

use std::sync::Arc;

use serde::Deserialize;
use tauri::{ipc::Channel, AppHandle, Runtime};
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
