//! iOS plugin glue. Methods here forward to the corresponding Swift
//! methods on `MpvBridgePlugin` via `run_mobile_plugin`. Names match the
//! `@objc` Swift selectors.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{
    ipc::Channel,
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::models::*;

// Payload shape expected by Tauri's built-in `registerListener` plugin
// method (see `mobile/ios-api/Sources/Tauri/Plugin/Plugin.swift`). The
// Swift base class owns a `listeners: [String: [Channel]]` dict and
// dispatches `trigger(_:data:)` calls to every matching channel.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterListenerArgs<'a> {
    event: &'a str,
    handler: Channel<serde_json::Value>,
}

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_ramus_ios_bridge);

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<RamusIosBridge<R>> {
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_ramus_ios_bridge)?;
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin("", "MpvBridgePlugin")?;
    Ok(RamusIosBridge(handle))
}

/// Handle on the Swift `MpvBridgePlugin`. All methods synchronously round
/// trip to Swift via `run_mobile_plugin`, which crosses the JS bridge on
/// iOS (~1ms per call).
pub struct RamusIosBridge<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> RamusIosBridge<R> {
    pub fn init_audio(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("initAudio", Empty::default())?;
        Ok(())
    }

    pub fn mpv_init(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvInit", Empty::default())?;
        Ok(())
    }

    pub fn mpv_load_file(&self, args: LoadFileArgs) -> crate::Result<()> {
        self.0.run_mobile_plugin::<Empty>("mpvLoadFile", args)?;
        Ok(())
    }

    pub fn mpv_load_file_at(&self, args: LoadFileAtArgs) -> crate::Result<()> {
        self.0.run_mobile_plugin::<Empty>("mpvLoadFileAt", args)?;
        Ok(())
    }

    pub fn mpv_playlist_play_index(&self, index: i64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvPlaylistPlayIndex", PlaylistIndexArgs { index })?;
        Ok(())
    }

    pub fn mpv_playlist_remove(&self, index: i64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvPlaylistRemove", PlaylistIndexArgs { index })?;
        Ok(())
    }

    pub fn mpv_playlist_move(&self, from: i64, to: i64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvPlaylistMove", PlaylistMoveArgs { from, to })?;
        Ok(())
    }

    pub fn mpv_seek(&self, position: f64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvSeek", SeekArgs { position })?;
        Ok(())
    }

    pub fn mpv_set_pause(&self, paused: bool) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvSetPause", PauseArgs { paused })?;
        Ok(())
    }

    pub fn mpv_set_volume(&self, volume: f64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvSetVolume", VolumeArgs { volume })?;
        Ok(())
    }

    pub fn mpv_get_volume(&self) -> crate::Result<f64> {
        let response: VolumeResponse = self.0.run_mobile_plugin("mpvGetVolume", Empty::default())?;
        Ok(response.volume)
    }

    pub fn mpv_set_audio_filters(&self, value: &str) -> crate::Result<()> {
        self.0.run_mobile_plugin::<Empty>(
            "mpvSetAudioFilters",
            AudioFiltersArgs {
                value: value.to_string(),
            },
        )?;
        Ok(())
    }

    pub fn mpv_stop(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("mpvStop", Empty::default())?;
        Ok(())
    }

    pub fn now_playing_update(&self, metadata: NowPlayingMetadata) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("nowPlayingUpdate", metadata)?;
        Ok(())
    }

    pub fn now_playing_clear(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<Empty>("nowPlayingClear", Empty::default())?;
        Ok(())
    }

    /// Read a keychain item. Returns `None` on miss (Swift side resolves
    /// missing items as an empty string, which we translate here so callers
    /// don't have to carry that convention).
    pub fn keychain_read(&self, account: &str) -> crate::Result<Option<String>> {
        let response: KeychainReadResponse = self.0.run_mobile_plugin(
            "keychainRead",
            KeychainAccountArgs {
                account: account.to_string(),
            },
        )?;
        if response.value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(response.value))
        }
    }

    pub fn keychain_write(&self, account: &str, value: &str) -> crate::Result<bool> {
        let response: KeychainBoolResponse = self.0.run_mobile_plugin(
            "keychainWrite",
            KeychainWriteArgs {
                account: account.to_string(),
                value: value.to_string(),
            },
        )?;
        Ok(response.ok)
    }

    pub fn keychain_delete(&self, account: &str) -> crate::Result<bool> {
        let response: KeychainBoolResponse = self.0.run_mobile_plugin(
            "keychainDelete",
            KeychainAccountArgs {
                account: account.to_string(),
            },
        )?;
        Ok(response.ok)
    }

    /// Attach a `Channel` to an event name that the Swift plugin emits
    /// via `trigger(_:data:)`. The channel's callback is invoked on
    /// every matching event with the JSON-serialised data. Used by the
    /// Rust-side event pump for mpv property changes and remote-command
    /// callbacks.
    pub fn register_listener(
        &self,
        event: &str,
        handler: Channel<serde_json::Value>,
    ) -> crate::Result<()> {
        self.0.run_mobile_plugin::<Empty>(
            "registerListener",
            RegisterListenerArgs { event, handler },
        )?;
        Ok(())
    }
}
