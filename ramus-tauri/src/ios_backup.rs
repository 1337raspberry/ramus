//! iOS backup exclusion for downloaded audio files.
//!
//! Downloaded tracks can total tens of GB on a device — they must not be
//! swept into iCloud / Finder backups. iOS exposes this via
//! `NSURLIsExcludedFromBackupKey` on the file's `URL`; the Swift plugin
//! sets it after the download lands.
//!
//! On desktop the call is a no-op.

use std::path::Path;
use std::sync::OnceLock;

pub trait BackupExcluder: Send + Sync {
    fn exclude(&self, path: &Path) -> Result<(), String>;
}

static BACKEND: OnceLock<Box<dyn BackupExcluder>> = OnceLock::new();

/// Register the platform backup backend. Called once at startup on iOS.
/// Desktop never calls this — `exclude_from_backup` becomes a silent no-op.
#[allow(dead_code)] // used on iOS; unused on desktop is expected.
pub fn register_backend(backend: Box<dyn BackupExcluder>) {
    let _ = BACKEND.set(backend);
}

/// Mark a file so the OS skips it during iCloud / iTunes backups. Silent
/// no-op if no backend was registered (desktop, or iOS before the plugin
/// initialised).
pub fn exclude_from_backup(path: &Path) {
    let Some(backend) = BACKEND.get() else {
        return;
    };
    if let Err(e) = backend.exclude(path) {
        log::warn!("ios_backup: failed to exclude {}: {e}", path.display());
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use super::{BackupExcluder, Path};
    use tauri::{AppHandle, Runtime};
    use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;

    pub struct IosBackup<R: Runtime> {
        app: AppHandle<R>,
    }

    impl<R: Runtime> IosBackup<R> {
        pub fn new(app: AppHandle<R>) -> Self {
            Self { app }
        }
    }

    impl<R: Runtime> BackupExcluder for IosBackup<R> {
        fn exclude(&self, path: &Path) -> Result<(), String> {
            let p = path.to_string_lossy();
            self.app
                .ramus_ios_bridge()
                .exclude_from_backup(&p)
                .map_err(|e| e.to_string())
                .and_then(|ok| {
                    if ok {
                        Ok(())
                    } else {
                        Err("plugin returned ok=false".into())
                    }
                })
        }
    }

    pub fn register<R: Runtime>(app: &AppHandle<R>) {
        super::register_backend(Box::new(IosBackup::new(app.clone())));
    }
}

#[cfg(target_os = "ios")]
pub use ios::register as register_ios;
