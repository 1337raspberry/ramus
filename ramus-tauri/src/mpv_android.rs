//! Android `MpvPlayer` implementation that delegates to the Kotlin
//! `MpvBridgePlugin` via Tauri mobile IPC.
//!
//! The Kotlin side owns the libmpv handle (loaded from `libmpv.so` +
//! `libplayer.so` JNI shim) and forwards property-change events through
//! `Plugin.trigger`. `crate::mpv_mobile::register_mpv_listeners` wires
//! those events back into `MpvCallbacks` — same plumbing as iOS, just a
//! different native backend behind the bridge.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Runtime};
use tauri_plugin_ramus_ios_bridge::{LoadFileArgs, LoadFileAtArgs, RamusIosBridgeExt};

use ramus_core::playback::mpv::{LoadMode, MpvCallbacks, MpvPlayer};

use crate::mpv_mobile::register_mpv_listeners;

pub struct AndroidMpvPlayer<R: Runtime> {
    app: AppHandle<R>,
    shutdown: Arc<AtomicBool>,
    cached_volume: AtomicU64,
}

impl<R: Runtime> AndroidMpvPlayer<R> {
    pub fn new(app: AppHandle<R>, callbacks: Arc<MpvCallbacks>) -> Result<Self, String> {
        // Register channels before mpv_init so any property-change events
        // that fire during `mpv_initialize` aren't dropped.
        register_mpv_listeners(&app, callbacks)
            .map_err(|e| format!("failed to register mpv listeners: {e}"))?;

        // Same ordering as iOS for consistency. Android's audio focus path
        // doesn't have the AVAudioSession sample-rate quirk, but matching
        // the order keeps the platforms easy to reason about.
        app.ramus_ios_bridge()
            .mpv_init()
            .map_err(|e| format!("failed to init mpv: {e}"))?;
        app.ramus_ios_bridge()
            .init_audio()
            .map_err(|e| format!("failed to init audio session: {e}"))?;

        Ok(Self {
            app,
            shutdown: Arc::new(AtomicBool::new(false)),
            cached_volume: AtomicU64::new(100.0_f64.to_bits()),
        })
    }

    fn bridge(&self) -> &tauri_plugin_ramus_ios_bridge::RamusIosBridge<R> {
        self.app.ramus_ios_bridge()
    }
}

impl<R: Runtime> MpvPlayer for AndroidMpvPlayer<R> {
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
        self.shutdown.load(Ordering::Acquire)
    }
}
