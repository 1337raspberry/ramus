//! iOS `MpvPlayer` implementation that delegates to the Swift
//! `MpvBridgePlugin` via Tauri mobile IPC.
//!
//! The Swift side owns the real mpv handle (loaded via MPVKit) and
//! forwards mpv's property-change events through `Plugin.trigger`.
//! `crate::mpv_mobile::register_mpv_listeners` wires those events back
//! into `MpvCallbacks`; this file only owns the per-platform construction
//! order (mpv_init BEFORE init_audio so AVAudioSession doesn't probe a
//! mismatched hardware sample rate — see CLAUDE.md).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Runtime};
use tauri_plugin_ramus_ios_bridge::{LoadFileArgs, LoadFileAtArgs, RamusIosBridgeExt};

use ramus_core::playback::mpv::{LoadMode, MpvCallbacks, MpvPlayer};

use crate::mpv_mobile::register_mpv_listeners;

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
