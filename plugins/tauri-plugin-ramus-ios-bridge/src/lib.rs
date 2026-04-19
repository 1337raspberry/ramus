//! Tauri iOS bridge for libmpv (via MPVKit) + `AVAudioSession` +
//! `MPNowPlayingInfoCenter` / `MPRemoteCommandCenter`.
//!
//! The plugin is declared as an unconditional Rust dependency of
//! `ramus-tauri`. On desktop it compiles to a no-op stub
//! (`desktop::RamusIosBridge`) so the app keeps using libloading +
//! souvlaki; on iOS it proxies method calls into a Swift `Plugin`
//! subclass that owns the mpv handle, the audio session, and the
//! lock-screen metadata.
//!
//! The plugin exposes only Rust callers — no JS commands — so the
//! permissions manifest is empty.

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use error::{Error, Result};
pub use models::*;

// Android has no Kotlin-side `MpvBridgePlugin` yet, so it reuses the
// desktop no-op stub — the plugin compiles but every call is a no-op.
// When the Android port gets a real bridge, swap this back to `cfg(mobile)`.
#[cfg(any(desktop, target_os = "android"))]
mod desktop;
#[cfg(target_os = "ios")]
mod mobile;

mod error;
mod models;

// Re-exported so consumers can type `tauri_plugin_ramus_ios_bridge::RamusIosBridge<R>`
// without caring which backend they're resolving against.
#[cfg(any(desktop, target_os = "android"))]
pub use desktop::RamusIosBridge;
#[cfg(target_os = "ios")]
pub use mobile::RamusIosBridge;

/// Extension trait for grabbing the bridge from any `Manager`.
pub trait RamusIosBridgeExt<R: Runtime> {
    fn ramus_ios_bridge(&self) -> &RamusIosBridge<R>;
}

impl<R: Runtime, T: Manager<R>> crate::RamusIosBridgeExt<R> for T {
    fn ramus_ios_bridge(&self) -> &RamusIosBridge<R> {
        self.state::<RamusIosBridge<R>>().inner()
    }
}

/// Register the plugin with Tauri. Call from the app's `run()` builder.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("ramus-ios-bridge")
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            let bridge = mobile::init(app, api)?;
            #[cfg(any(desktop, target_os = "android"))]
            let bridge = desktop::init(app, api)?;
            app.manage(bridge);
            Ok(())
        })
        .build()
}
