//! C ABI shims for the platform voice-assistant / App Intents integration.
//!
//! The native intent layer (compiled into the app target) calls these to answer
//! natural-language library questions and to start playback. Two seams:
//!
//!   * **Read-only** (`ramus_siri_probe`, `ramus_siri_genres`,
//!     `ramus_siri_artists`) — all real work lives in
//!     [`ramus_core::siri_probe`], which opens its own read-only view of the
//!     on-disk cache so an intent can run without the app's live state.
//!   * **Playback** (`ramus_siri_play`) — needs the *live* player (mpv bridge),
//!     so it reaches the running app through a stashed [`AppHandle`] rather than
//!     a fresh DB connection. The play intent foregrounds the app
//!     (`openAppWhenRun`), so the handle is populated by the time this runs.
//!
//! All declared in the Swift bridging header as plain C.

use std::ffi::{c_char, CStr, CString};
use std::sync::OnceLock;
use std::time::Duration;

use rand::seq::SliceRandom;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use ramus_core::models::Track;

use crate::state::AppState;

/// Live app handle, stashed once during `run().setup()` after the app state is
/// managed and the player is configured. Its presence is the readiness signal
/// the play path waits on. Read-only intents never touch this.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Register the running app so playback intents can reach the live player.
/// Called once, late in setup (after `app.manage(state)`); a second call is a
/// no-op.
pub fn set_app_handle(handle: AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

// --- C-string plumbing ------------------------------------------------------

/// Borrow a nullable C string as an owned `Option<String>` (null → `None`).
///
/// # Safety
/// `ptr` must be null or a valid pointer to a NUL-terminated C string.
unsafe fn cstr_to_opt(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        CStr::from_ptr(ptr).to_str().ok().map(str::to_owned)
    }
}

/// Hand a JSON string to the caller as a heap-allocated C string it must later
/// release with [`ramus_siri_free`]. Null only on an interior NUL (never, in
/// practice — the payload is JSON).
fn into_raw_json(json: String) -> *mut c_char {
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// --- Read-only intents ------------------------------------------------------

/// Answer a library probe for the given genre (nullable → no genre filter).
///
/// # Safety
/// `genre` must be null or a valid pointer to a NUL-terminated C string. The
/// returned pointer must be released with [`ramus_siri_free`].
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_probe(genre: *const c_char) -> *mut c_char {
    let genre = cstr_to_opt(genre);
    into_raw_json(ramus_core::siri_probe::probe_json(genre.as_deref()))
}

/// List the genres present in the library (JSON `{ok, items:[{name,albumCount}]}`).
/// `query` is an optional case-insensitive substring filter (null → the top
/// suggestions). Backs the assistant's genre entity vocabulary.
///
/// # Safety
/// `query` must be null or a valid pointer to a NUL-terminated C string. The
/// returned pointer must be released with [`ramus_siri_free`].
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_genres(query: *const c_char) -> *mut c_char {
    let query = cstr_to_opt(query);
    into_raw_json(ramus_core::siri_probe::list_genres_json(query.as_deref()))
}

/// List the artists present in the library (JSON `{ok, items:[{name}]}`).
/// `query` is an optional case-insensitive substring filter (null → the top
/// suggestions). Backs the assistant's artist entity vocabulary.
///
/// # Safety
/// `query` must be null or a valid pointer to a NUL-terminated C string. The
/// returned pointer must be released with [`ramus_siri_free`].
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_artists(query: *const c_char) -> *mut c_char {
    let query = cstr_to_opt(query);
    into_raw_json(ramus_core::siri_probe::list_artists_json(query.as_deref()))
}

/// List the albums present in the library (JSON `{ok, items:[{sourceId, title,
/// artist}]}`). `query` is an optional case-insensitive title filter (null → the
/// top suggestions). Backs the assistant's album vocabulary — an album is a
/// playable song collection.
///
/// # Safety
/// `query` must be null or a valid pointer to a NUL-terminated C string. The
/// returned pointer must be released with [`ramus_siri_free`].
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_albums(query: *const c_char) -> *mut c_char {
    let query = cstr_to_opt(query);
    into_raw_json(ramus_core::siri_probe::list_albums_json(query.as_deref()))
}

// --- Playback intent --------------------------------------------------------

/// Result of a play request, serialised for the native layer. The assistant
/// speaks `spoken`; the rest is for on-device diagnostics.
#[derive(Debug, Serialize)]
struct PlayResult {
    ok: bool,
    spoken: String,
    track_count: usize,
    error: Option<String>,
}

impl PlayResult {
    fn played(spoken: String, count: usize) -> Self {
        Self {
            ok: true,
            spoken,
            track_count: count,
            error: None,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            spoken: message.clone(),
            track_count: 0,
            error: Some(message),
        }
    }
}

/// Start playback of a genre, artist, or album, reaching the live player through
/// the stashed handle. `album` is a Plex source id (rating key), not a title.
/// Returns JSON `{ok, spoken, trackCount}`.
///
/// # Safety
/// `genre`, `artist`, and `album` must each be null or a valid pointer to a
/// NUL-terminated C string. The returned pointer must be released with
/// [`ramus_siri_free`].
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_play(
    genre: *const c_char,
    artist: *const c_char,
    album: *const c_char,
) -> *mut c_char {
    let genre = cstr_to_opt(genre);
    let artist = cstr_to_opt(artist);
    let album = cstr_to_opt(album);
    let result = play(genre.as_deref(), artist.as_deref(), album.as_deref());
    let json = serde_json::to_string(&result).unwrap_or_else(|_| {
        "{\"ok\":false,\"spoken\":\"ramus couldn't start playback.\"}".to_string()
    });
    into_raw_json(json)
}

/// Wait briefly for the app to be ready to play. The play intent foregrounds the
/// app, which may be cold-launching, so poll until the handle is registered and
/// the cache is initialised (i.e. the player is configured). Runs on the
/// intent's own thread, so a blocking wait is fine.
fn wait_for_ready() -> Option<AppHandle> {
    for _ in 0..40 {
        if let Some(handle) = APP_HANDLE.get() {
            let ready = handle
                .try_state::<AppState>()
                .map(|state| state.cache.lock().is_some())
                .unwrap_or(false);
            if ready {
                return Some(handle.clone());
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Last resort: return whatever handle we have so the intent can still try.
    APP_HANDLE.get().cloned()
}

fn play(genre: Option<&str>, artist: Option<&str>, album: Option<&str>) -> PlayResult {
    let handle = match wait_for_ready() {
        Some(handle) => handle,
        None => return PlayResult::failed("Open ramus and sign in, then try again."),
    };

    tauri::async_runtime::block_on(async move {
        // Resolve to a concrete queue first — this drops every DB lock before
        // the playback await (parking_lot guards can't cross an await point).
        let tracks = resolve_queue(&handle.state::<AppState>(), genre, artist, album);
        let count = tracks.len();
        if tracks.is_empty() {
            return PlayResult::failed(not_found_message(genre, artist, album));
        }

        match crate::commands::playback::play_tracks(
            handle.clone(),
            handle.state::<AppState>(),
            tracks,
            0,
        )
        .await
        {
            Ok(()) => PlayResult::played(playing_message(genre, artist, album, count), count),
            Err(e) => PlayResult::failed(format!("ramus couldn't start playback ({e}).")),
        }
    })
}

/// Resolve a genre, artist, or album to an ordered play queue. Specificity wins:
/// an album (a single record) beats an artist, which beats a genre. An album
/// plays its own tracks in order; an artist plays in album order; a genre is
/// shuffled across matching albums for a "play some X" mix.
fn resolve_queue(
    state: &AppState,
    genre: Option<&str>,
    artist: Option<&str>,
    album: Option<&str>,
) -> Vec<Track> {
    if let Some(album) = album {
        // `album` is a source id (rating key), so this is an exact lookup.
        let lock = state.cache.lock();
        let Some(db) = lock.as_ref() else {
            return Vec::new();
        };
        db.tracks_for_album(album).unwrap_or_default()
    } else if let Some(artist) = artist {
        let lock = state.cache.lock();
        let Some(db) = lock.as_ref() else {
            return Vec::new();
        };
        let albums = db.albums_for_artist_name(artist).unwrap_or_default();
        albums
            .iter()
            .flat_map(|a| db.tracks_for_album(&a.rating_key).unwrap_or_default())
            .collect()
    } else if let Some(genre) = genre {
        // Resolve the genre to library tags BEFORE locking the cache — the
        // helper takes the cache lock itself (parking_lot is not re-entrant).
        let names = crate::commands::library::library_genre_names(state, genre)
            .unwrap_or_else(|_| vec![genre.to_string()]);
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

        let lock = state.cache.lock();
        let Some(db) = lock.as_ref() else {
            return Vec::new();
        };
        let albums = db.albums_for_genres(&name_refs).unwrap_or_default();
        let mut tracks: Vec<Track> = albums
            .iter()
            .flat_map(|a| db.tracks_for_album(&a.rating_key).unwrap_or_default())
            .collect();
        drop(lock);
        tracks.shuffle(&mut rand::thread_rng());
        tracks
    } else {
        Vec::new()
    }
}

fn not_found_message(genre: Option<&str>, artist: Option<&str>, album: Option<&str>) -> String {
    match (album, artist, genre) {
        (Some(_), _, _) => "I couldn't find that album in your ramus library.".to_string(),
        (None, Some(artist), _) => {
            format!("I couldn't find anything by {artist} in your ramus library.")
        }
        (None, None, Some(genre)) => {
            format!("I couldn't find any {genre} to play in your ramus library.")
        }
        (None, None, None) => "Tell me a genre, artist, or album to play.".to_string(),
    }
}

fn playing_message(
    genre: Option<&str>,
    artist: Option<&str>,
    album: Option<&str>,
    count: usize,
) -> String {
    match (album, artist, genre) {
        (Some(_), _, _) => "Playing that album in ramus.".to_string(),
        (None, Some(artist), _) => format!("Playing {artist} in ramus."),
        (None, None, Some(genre)) => format!("Playing {count} {genre} tracks in ramus."),
        (None, None, None) => "Playing your library in ramus.".to_string(),
    }
}

/// Release a string previously returned by any `ramus_siri_*` function.
///
/// # Safety
/// `ptr` must be null, or a pointer returned by a `ramus_siri_*` function that
/// has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

// --- iOS scene-window bridge ------------------------------------------------
//
// The windowing layer (tao) predates the iOS scene lifecycle: it creates its
// `UIWindow` from the app delegate and never assigns a `windowScene`. A
// scene-less window is invisible AND absent from `UIApplication.windows`, so the
// app's own scene delegate has no way to find it (that gap is a black screen on
// the iOS 26+ SDK, which makes scene adoption mandatory). We stash the webview's
// view controller during setup — where the framework hands it to us — and expose
// the enclosing window so the scene delegate can adopt it into the connected
// scene. The window is reached the same way as the startup resize: walk the view
// controller's root view up to its `window`.

#[cfg(target_os = "ios")]
use std::sync::atomic::{AtomicUsize, Ordering};

/// The main webview's `UIViewController*`, stashed during `run().setup()`. Zero
/// until then. Held as `usize` because a raw pointer isn't `Sync`.
#[cfg(target_os = "ios")]
static MAIN_VIEW_CONTROLLER: AtomicUsize = AtomicUsize::new(0);

/// Record the main webview's view controller so [`ramus_ios_main_window`] can
/// resolve the enclosing window on demand. Called once, from setup.
#[cfg(target_os = "ios")]
pub fn set_main_view_controller(ptr: usize) {
    MAIN_VIEW_CONTROLLER.store(ptr, Ordering::SeqCst);
}

/// Return the main `UIWindow*` (an ObjC `id`) for the scene delegate to adopt
/// into the active `UIWindowScene`. Null until the view controller is stashed
/// and it has a window; the caller is expected to retry. Must run on the main
/// thread (it messages UIKit objects).
///
/// # Safety
/// The returned pointer is an unretained `UIWindow*` owned by the windowing
/// layer — do not release it, and use it only on the main thread.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn ramus_ios_main_window() -> *mut std::ffi::c_void {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let vc = MAIN_VIEW_CONTROLLER.load(Ordering::SeqCst) as *mut AnyObject;
    if vc.is_null() {
        return std::ptr::null_mut();
    }
    let root_view: *mut AnyObject = msg_send![vc, view];
    if root_view.is_null() {
        return std::ptr::null_mut();
    }
    let win: *mut AnyObject = msg_send![root_view, window];
    win.cast()
}
