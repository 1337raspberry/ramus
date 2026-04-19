pub type CmdResult<T> = Result<T, String>;

pub mod acknowledgements;
pub mod auth;
pub mod downloads;
pub mod library;
pub mod platform;
pub mod playback;
pub mod search;
pub mod settings;
pub mod spectrum;
pub mod sync;

/// Lock the cache DB and invoke `f`. Returns an IPC-friendly error string
/// when the cache isn't initialised yet (pre-onboarding / pre-session-restore).
pub(super) fn with_cache<F, T>(state: &crate::state::AppState, f: F) -> CmdResult<T>
where
    F: FnOnce(
        &ramus_core::cache::db::CacheDatabase,
    ) -> Result<T, ramus_core::cache::db::CacheError>,
{
    let lock = state.cache.lock();
    let db = lock.as_ref().ok_or("Cache not initialized")?;
    f(db).map_err(|e| e.to_string())
}
