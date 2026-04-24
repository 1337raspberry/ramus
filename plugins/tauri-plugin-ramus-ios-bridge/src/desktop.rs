//! Desktop stub. The plugin has no desktop behaviour — libmpv is loaded
//! directly via `libloading` on desktop, and souvlaki handles media
//! controls. Every method here is a no-op so the main app crate can
//! depend on the plugin unconditionally without cfg gates.

use serde::de::DeserializeOwned;
use tauri::{ipc::Channel, plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<RamusIosBridge<R>> {
    Ok(RamusIosBridge(app.clone()))
}

pub struct RamusIosBridge<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

impl<R: Runtime> RamusIosBridge<R> {
    pub fn init_audio(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_init(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_load_file(&self, _args: LoadFileArgs) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_load_file_at(&self, _args: LoadFileAtArgs) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_playlist_play_index(&self, _index: i64) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_playlist_remove(&self, _index: i64) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_playlist_move(&self, _from: i64, _to: i64) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_seek(&self, _position: f64) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_set_pause(&self, _paused: bool) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_set_volume(&self, _volume: f64) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_get_volume(&self) -> crate::Result<f64> {
        Ok(100.0)
    }
    pub fn mpv_get_eq_config(&self) -> crate::Result<EqConfigResponse> {
        Ok(EqConfigResponse {
            frequencies: vec![31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000],
            min_gain: -12.0,
            max_gain: 12.0,
        })
    }
    pub fn mpv_set_audio_filters(&self, _value: &str) -> crate::Result<()> {
        Ok(())
    }
    pub fn mpv_stop(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn dismiss_keyboard(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn show_native_search_bar(&self, _initial_query: &str) -> crate::Result<()> {
        Ok(())
    }
    pub fn hide_native_search_bar(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn now_playing_update(&self, _metadata: NowPlayingMetadata) -> crate::Result<()> {
        Ok(())
    }
    pub fn now_playing_clear(&self) -> crate::Result<()> {
        Ok(())
    }
    pub fn set_media_accent(&self, _r: u8, _g: u8, _b: u8) -> crate::Result<()> {
        Ok(())
    }
    pub fn keychain_read(&self, _account: &str) -> crate::Result<Option<String>> {
        Ok(None)
    }
    pub fn keychain_write(&self, _account: &str, _value: &str) -> crate::Result<bool> {
        Ok(false)
    }
    pub fn keychain_delete(&self, _account: &str) -> crate::Result<bool> {
        Ok(false)
    }
    pub fn exclude_from_backup(&self, _path: &str) -> crate::Result<bool> {
        Ok(true)
    }
    pub fn register_listener(
        &self,
        _event: &str,
        _handler: Channel<serde_json::Value>,
    ) -> crate::Result<()> {
        Ok(())
    }
}
