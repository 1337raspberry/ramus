//! iOS `KeychainBackend` implementation that forwards to the Swift
//! `KeychainBridge` through the `ramus-ios-bridge` plugin.
//!
//! Registered with `ramus-core::plex::token_store::set_keychain_backend`
//! at app startup so subsequent `TokenStore::new()` calls see the backend.

use std::sync::Arc;

use tauri::{AppHandle, Runtime};
use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;

use ramus_core::plex::token_store::KeychainBackend;

pub struct IosKeychain<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> IosKeychain<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> KeychainBackend for IosKeychain<R> {
    fn read(&self, account: &str) -> Option<String> {
        self.app
            .ramus_ios_bridge()
            .keychain_read(account)
            .unwrap_or_else(|e| {
                log::warn!("keychain_read({account}) failed: {e}");
                None
            })
    }

    fn write(&self, account: &str, value: &str) -> bool {
        match self.app.ramus_ios_bridge().keychain_write(account, value) {
            Ok(ok) => ok,
            Err(e) => {
                log::warn!("keychain_write({account}) failed: {e}");
                false
            }
        }
    }

    fn delete(&self, account: &str) -> bool {
        match self.app.ramus_ios_bridge().keychain_delete(account) {
            Ok(ok) => ok,
            Err(e) => {
                log::warn!("keychain_delete({account}) failed: {e}");
                false
            }
        }
    }
}

pub fn register<R: Runtime>(app: &AppHandle<R>) {
    ramus_core::plex::token_store::set_keychain_backend(Arc::new(IosKeychain::new(app.clone())));
}
