//! iOS `MpvPlayer` implementation that delegates to the Swift
//! `MpvBridgePlugin` via Tauri mobile IPC.
//!
//! The Swift side owns the real mpv handle (loaded via MPVKit) and
//! forwards mpv's property-change events through `Plugin.trigger`. The
//! base `Plugin` class routes those to any registered `Channel`s, so
//! this module registers one channel per event at startup and converts
//! each incoming JSON payload into the corresponding `MpvCallbacks`
//! invocation.
//!
//! Command methods (load/seek/pause/…) round-trip synchronously through
//! `run_mobile_plugin`; event plumbing is one-way plugin→Rust via the
//! channels.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use tauri::{ipc::Channel, AppHandle, Runtime};
use tauri_plugin_ramus_ios_bridge::{LoadFileArgs, LoadFileAtArgs, RamusIosBridgeExt};

use ramus_core::playback::mpv::{FileEndReason, LoadMode, MpvCallbacks, MpvPlayer};

pub struct IosMpvPlayer<R: Runtime> {
    app: AppHandle<R>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    cached_volume: AtomicU64,
}

impl<R: Runtime> IosMpvPlayer<R> {
    pub fn new(app: AppHandle<R>, callbacks: Arc<MpvCallbacks>) -> Result<Self, String> {
        // Register event channels BEFORE mpv_init so early property-change
        // events (e.g. idle-active during mpv_initialize) aren't dropped.
        register_mpv_listeners(&app, callbacks)
            .map_err(|e| format!("failed to register mpv listeners: {e}"))?;

        // mpv_initialize first, THEN AVAudioSession.setActive. Reversing
        // this makes `ao=audiounit` probe a different hardware sample rate
        // — audible as ~8.8% fast playback (48k driving 44.1k content).
        app.ramus_ios_bridge()
            .mpv_init()
            .map_err(|e| format!("failed to init mpv: {e}"))?;
        app.ramus_ios_bridge()
            .init_audio()
            .map_err(|e| format!("failed to init audio session: {e}"))?;

        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Ok(Self {
            app,
            shutdown,
            cached_volume: AtomicU64::new(100.0_f64.to_bits()),
        })
    }

    fn bridge(&self) -> &tauri_plugin_ramus_ios_bridge::RamusIosBridge<R> {
        self.app.ramus_ios_bridge()
    }
}

impl<R: Runtime> MpvPlayer for IosMpvPlayer<R> {
    fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>) {
        if let Err(e) = self.bridge().mpv_load_file(LoadFileArgs {
            url: url.to_string(),
            mode: mode.as_str().to_string(),
            options: options.map(|s| s.to_string()),
        }) {
            log::error!("mpv_load_file failed: {e}");
        }
    }

    fn load_file_at(&self, url: &str, index: i64, options: Option<&str>) {
        if let Err(e) = self.bridge().mpv_load_file_at(LoadFileAtArgs {
            url: url.to_string(),
            index,
            options: options.map(|s| s.to_string()),
        }) {
            log::error!("mpv_load_file_at failed: {e}");
        }
    }

    fn playlist_play_index(&self, index: i64) {
        let _ = self.bridge().mpv_playlist_play_index(index);
    }

    fn playlist_remove(&self, index: i64) {
        let _ = self.bridge().mpv_playlist_remove(index);
    }

    fn playlist_move(&self, from: i64, to: i64) {
        let _ = self.bridge().mpv_playlist_move(from, to);
    }

    fn seek(&self, position: f64) {
        let _ = self.bridge().mpv_seek(position);
    }

    fn set_pause(&self, paused: bool) {
        let _ = self.bridge().mpv_set_pause(paused);
    }

    fn set_volume(&self, volume: f64) {
        self.cached_volume
            .store(volume.to_bits(), Ordering::Relaxed);
        let _ = self.bridge().mpv_set_volume(volume);
    }

    fn get_volume(&self) -> f64 {
        f64::from_bits(self.cached_volume.load(Ordering::Relaxed))
    }

    fn set_audio_filters(&self, value: &str) {
        let _ = self.bridge().mpv_set_audio_filters(value);
    }

    fn stop(&self) {
        let _ = self.bridge().mpv_stop();
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(std::sync::atomic::Ordering::Acquire)
    }
}

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

/// Register one `Channel` per mpv event and wire each channel's
/// callback to the matching entry on `MpvCallbacks`. Channels live for
/// the process lifetime — the `AudioPlayer` singleton pins them.
fn register_mpv_listeners<R: Runtime>(
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
