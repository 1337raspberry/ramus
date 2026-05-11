//! libmpv controller implementing the MpvPlayer trait.
//!
//! Creates an mpv instance for audio-only playback, runs an event loop on a
//! background thread, and dispatches callbacks to the caller.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use ramus_core::playback::mpv::{FileEndReason, LoadMode, MpvCallbacks, MpvPlayer, ObserverID};
use ramus_core::util::redact_urls;

use crate::mpv_ffi::*;

struct MpvHandle(*mut mpv_handle);
unsafe impl Send for MpvHandle {}
unsafe impl Sync for MpvHandle {}

impl MpvHandle {
    fn ptr(&self) -> *mut mpv_handle {
        self.0
    }
}

pub struct MpvController {
    lib: Arc<MpvLib>,
    handle: Arc<MpvHandle>,
    shutdown: Arc<AtomicBool>,
    /// mpv 0.38 changed `loadfile` from `<url> <flags> [<options>]` to
    /// `<url> <flags> [<index>] [<options>]` and added the `insert-at` /
    /// `insert-next` flag values. Older libmpv (Ubuntu 24.04 LTS ships
    /// 0.35.1) rejects both. Probed once at init from `mpv-version`.
    loadfile_has_index_slot: bool,
    _event_thread: Option<thread::JoinHandle<()>>,
}

impl MpvController {
    /// Create and initialize a new mpv instance with a background event loop
    /// thread that dispatches `callbacks`.
    ///
    /// `lib` is the runtime-loaded libmpv; load it once at startup via
    /// `MpvLib::load()` and share the `Arc` across controllers.
    pub fn new(lib: Arc<MpvLib>, callbacks: Arc<MpvCallbacks>) -> Result<Self, String> {
        unsafe {
            // mpv requires LC_NUMERIC=C for POSIX float formatting (e.g. EQ filters).
            // Without this, mpv_create() returns null on Linux with non-C locales.
            #[cfg(target_os = "linux")]
            {
                let c_locale = std::ffi::CString::new("C").unwrap();
                let lc_numeric = 1; // LC_NUMERIC
                libc::setlocale(lc_numeric, c_locale.as_ptr());
            }

            let ctx = lib.create();
            if ctx.is_null() {
                return Err("mpv_create() returned null".into());
            }

            let options = ramus_core::playback::mpv::default_mpv_options();
            for (key, val) in &options {
                let k = CString::new(*key).unwrap();
                let v = CString::new(*val).unwrap();
                lib.set_option_string(ctx, k.as_ptr(), v.as_ptr());
            }

            let err = lib.initialize(ctx);
            if err < 0 {
                let msg = CStr::from_ptr(lib.error_string(err));
                lib.destroy(ctx);
                return Err(format!("mpv_initialize failed: {}", msg.to_string_lossy()));
            }

            let props = ramus_core::playback::mpv::observed_properties();
            for (name, id) in &props {
                let n = CString::new(*name).unwrap();
                let fmt = match id {
                    ObserverID::TimePos | ObserverID::Duration => MPV_FORMAT_DOUBLE,
                    ObserverID::Pause | ObserverID::IdleActive => MPV_FORMAT_FLAG,
                    ObserverID::PlaylistPos => MPV_FORMAT_INT64,
                };
                lib.observe_property(ctx, *id as u64, n.as_ptr(), fmt);
            }

            // Route mpv's own log messages into our log facade. Without
            // this, mpv-side errors (HTTP failures, demuxer issues, lavf
            // diagnostics, etc.) are completely invisible — the only
            // signal we get is `mpv_event_end_file.error`, which translates
            // to coarse strings like "loading failed" with no context.
            // `info` covers HTTP/lavf connection lifecycle (connect, open,
            // status code, premature close) which mpv emits at info-level,
            // not warn — without it the actual failure reason for a slow-
            // connection transcode bail is invisible. Override via
            // `RAMUS_MPV_LOG_LEVEL` env var if more / less is needed
            // (valid values: no, fatal, error, warn, info, v, debug, trace).
            let level_str = std::env::var("RAMUS_MPV_LOG_LEVEL")
                .unwrap_or_else(|_| "info".into());
            let level = CString::new(level_str).unwrap();
            lib.request_log_messages(ctx, level.as_ptr());

            // 100 = unity gain.
            let vol_name = CString::new("volume").unwrap();
            let mut vol: f64 = 100.0;
            lib.set_property(
                ctx,
                vol_name.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut vol as *mut f64 as *mut c_void,
            );

            let handle = Arc::new(MpvHandle(ctx));
            let shutdown = Arc::new(AtomicBool::new(false));

            // Probe libmpv's loadfile signature. Ubuntu 24.04 LTS ships
            // mpv 0.35.1 which expects `loadfile <url> <flags> [<options>]`
            // and only accepts replace/append/append-play in <flags>. mpv
            // 0.38+ added a positional <index> slot and the insert-at
            // flag. Without this branch we'd send `-1` in slot 3 on old
            // mpv, which gets parsed as the options string and fails.
            let loadfile_has_index_slot = match read_string_property(&lib, ctx, "mpv-version") {
                Some(v) => {
                    let new_signature = mpv_version_at_least(&v, 0, 38);
                    log::info!(
                        "libmpv: {v} (loadfile {} index slot)",
                        if new_signature { "has" } else { "lacks" }
                    );
                    new_signature
                }
                None => {
                    log::warn!(
                        "libmpv: could not read mpv-version, assuming pre-0.38 loadfile signature"
                    );
                    false
                }
            };

            let handle_clone = handle.clone();
            let shutdown_clone = shutdown.clone();
            let lib_clone = lib.clone();
            let event_thread = thread::Builder::new()
                .name("mpv-event-loop".into())
                .spawn(move || {
                    event_loop(lib_clone, handle_clone, shutdown_clone, callbacks);
                })
                .map_err(|e| format!("Failed to spawn mpv event thread: {e}"))?;

            Ok(Self {
                lib,
                handle,
                shutdown,
                loadfile_has_index_slot,
                _event_thread: Some(event_thread),
            })
        }
    }

    fn command(&self, args: &[&str]) {
        // CString::new errors on interior NULs. Args here include URLs and
        // file paths sourced from Plex responses, so a malformed entry
        // would otherwise panic the calling thread. Skip the command and
        // log only the position to avoid leaking token-bearing URLs.
        let c_args: Vec<CString> = match args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(v) => v,
            Err(e) => {
                log::error!(
                    "mpv command rejected: argument contains NUL byte at position {}",
                    e.nul_position()
                );
                return;
            }
        };
        unsafe {
            let mut ptrs: Vec<*const c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
            ptrs.push(std::ptr::null());
            self.lib.command(self.handle.ptr(), ptrs.as_ptr());
        }
    }

    fn set_property_double(&self, name: &str, value: f64) {
        unsafe {
            let n = CString::new(name).unwrap();
            let mut v = value;
            self.lib.set_property(
                self.handle.ptr(),
                n.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            );
        }
    }

    fn set_property_flag(&self, name: &str, value: bool) {
        unsafe {
            let n = CString::new(name).unwrap();
            let mut v: c_int = if value { 1 } else { 0 };
            self.lib.set_property(
                self.handle.ptr(),
                n.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut v as *mut c_int as *mut c_void,
            );
        }
    }

    fn get_property_int64(&self, name: &str) -> Option<i64> {
        unsafe {
            let n = CString::new(name).ok()?;
            let mut v: i64 = 0;
            let ret = self.lib.get_property(
                self.handle.ptr(),
                n.as_ptr(),
                MPV_FORMAT_INT64,
                &mut v as *mut i64 as *mut c_void,
            );
            if ret < 0 {
                return None;
            }
            Some(v)
        }
    }

    fn get_property_double(&self, name: &str) -> Option<f64> {
        unsafe {
            let n = CString::new(name).ok()?;
            let mut v: f64 = 0.0;
            // libmpv leaves the out-buffer in an unspecified state on error
            // (return < 0). Without this check, get_volume() would silently
            // return 0.0 on any read failure and the caller could commit it
            // back as the real volume — silent mute on a transient error.
            let ret = self.lib.get_property(
                self.handle.ptr(),
                n.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            );
            if ret < 0 {
                return None;
            }
            Some(v)
        }
    }
}

/// Read a string-typed mpv property. libmpv allocates the C string and the
/// caller must free it via `mpv_free` — wrapping the unsafe dance here keeps
/// it confined.
///
/// Free-function (not a method) so the loadfile-signature probe in `new()`
/// can run before the `Self` is built.
fn read_string_property(lib: &MpvLib, ctx: *mut mpv_handle, name: &str) -> Option<String> {
    unsafe {
        let n = CString::new(name).ok()?;
        let mut out: *mut c_char = std::ptr::null_mut();
        let ret = lib.get_property(
            ctx,
            n.as_ptr(),
            MPV_FORMAT_STRING,
            &mut out as *mut *mut c_char as *mut c_void,
        );
        if ret < 0 || out.is_null() {
            return None;
        }
        let value = CStr::from_ptr(out).to_string_lossy().into_owned();
        lib.free(out as *mut c_void);
        Some(value)
    }
}

/// Parse mpv's `mpv-version` string and return whether the reported version
/// is at least `major.minor`. Real-world formats observed:
/// - `"mpv 0.35.1"`            — Ubuntu/Debian apt packages
/// - `"mpv v0.41.0"`           — Homebrew on macOS (leading `v`)
/// - `"mpv 0.40.0-1"`          — Debian patch suffix
/// - `"mpv 0.38.0-rc1"`        — pre-release tag
/// - `"mpv git-deadbeef"`      — self-built from git
///
/// Strategy: strip the `"mpv "` prefix, skip any leading non-digit
/// characters (handles the `v` prefix), then parse `major.minor` up to
/// the first non-version character.
///
/// Returns `false` on any parse failure — safe default is the older
/// signature, since old mpv rejects the new args outright while new mpv
/// tolerates the 3-arg `loadfile` with no per-track options (see
/// `load_file` below). The bias is therefore "fail safe on old mpv,"
/// not "fail safe on new mpv" — getting it wrong on a new mpv breaks
/// stream-record (loadfile would put options in the index slot).
fn mpv_version_at_least(version: &str, min_major: u32, min_minor: u32) -> bool {
    let Some(rest) = version.strip_prefix("mpv ") else {
        return false;
    };
    let after_v = rest.trim_start_matches(|c: char| !c.is_ascii_digit());
    let numeric = after_v
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()
        .unwrap_or("");
    let mut parts = numeric.split('.');
    let Some(major) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
        return false;
    };
    (major, minor) >= (min_major, min_minor)
}

impl MpvPlayer for MpvController {
    fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>) {
        // loadfile arg layout depends on libmpv version (probed at init).
        // mpv 0.38+ : `loadfile <url> <flags> [<index>] [<options>]` — pass
        //             "-1" in the index slot as the "no index" sentinel.
        // mpv <0.38: `loadfile <url> <flags> [<options>]` — no index slot;
        //             passing "-1" gets parsed as the options string and
        //             fails with "Expected '=' and a value".
        //
        // Note: `replace` implicitly stops; callers must not invoke stop()
        // before load_queue or they race with playlist setup.
        match (options, self.loadfile_has_index_slot) {
            (Some(opts), true) => self.command(&["loadfile", url, mode.as_str(), "-1", opts]),
            (Some(opts), false) => self.command(&["loadfile", url, mode.as_str(), opts]),
            (None, _) => self.command(&["loadfile", url, mode.as_str()]),
        }
    }

    fn load_file_at(&self, url: &str, index: i64, options: Option<&str>) {
        if self.loadfile_has_index_slot {
            let idx = index.to_string();
            match options {
                Some(opts) => self.command(&["loadfile", url, "insert-at", &idx, opts]),
                None => self.command(&["loadfile", url, "insert-at", &idx]),
            }
            return;
        }

        // Pre-0.38 mpv has no `insert-at` flag. Emulate by appending to
        // the end of the playlist and then moving the new entry into
        // position. `playlist-count` is read AFTER the append so the
        // index of the appended entry is unambiguous even under
        // concurrent playlist mutations (none today, but future-proofing
        // is cheap). `from > to` works for backward moves; `from == to`
        // is a no-op which mpv tolerates.
        match options {
            Some(opts) => self.command(&["loadfile", url, "append", opts]),
            None => self.command(&["loadfile", url, "append"]),
        }
        match self.get_property_int64("playlist-count") {
            Some(count) if count > 0 => {
                self.playlist_move(count - 1, index);
            }
            _ => {
                log::error!(
                    "load_file_at: playlist-count unreadable on pre-0.38 mpv; \
                     appended entry left at end instead of index {index}"
                );
            }
        }
    }

    fn playlist_play_index(&self, index: i64) {
        unsafe {
            let name = CString::new("playlist-pos").unwrap();
            let mut v = index;
            self.lib.set_property(
                self.handle.ptr(),
                name.as_ptr(),
                MPV_FORMAT_INT64,
                &mut v as *mut i64 as *mut c_void,
            );
        }
    }

    fn playlist_remove(&self, index: i64) {
        self.command(&["playlist-remove", &index.to_string()]);
    }

    fn playlist_move(&self, from: i64, to: i64) {
        self.command(&["playlist-move", &from.to_string(), &to.to_string()]);
    }

    fn seek(&self, position: f64) {
        self.command(&["seek", &format!("{position:.3}"), "absolute"]);
    }

    fn set_pause(&self, paused: bool) {
        self.set_property_flag("pause", paused);
    }

    fn set_volume(&self, volume: f64) {
        self.set_property_double("volume", volume);
    }

    fn get_volume(&self) -> f64 {
        // 100.0 is libmpv's default volume; safer fallback than 0 because the
        // caller may write this value back via set_volume() during state
        // restoration and a 0 reading would silently mute audio.
        self.get_property_double("volume").unwrap_or(100.0)
    }

    fn set_audio_filters(&self, value: &str) {
        let val = match CString::new(value) {
            Ok(v) => v,
            Err(_) => {
                log::error!("mpv set_audio_filters rejected: value contains NUL byte");
                return;
            }
        };
        unsafe {
            let name = CString::new("af").unwrap();
            self.lib
                .set_property_string(self.handle.ptr(), name.as_ptr(), val.as_ptr());
        }
    }

    fn stop(&self) {
        self.command(&["stop"]);
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    fn demuxer_cache_time(&self) -> Option<f64> {
        self.get_property_double("demuxer-cache-time")
    }
}

impl Drop for MpvController {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock mpv_wait_event so the event loop thread exits.
        self.command(&["quit"]);
        if let Some(t) = self._event_thread.take() {
            let _ = t.join();
        }
        unsafe {
            self.lib.destroy(self.handle.ptr());
        }
    }
}

/// Invoke a caller-supplied callback, catching any panic so a single bad
/// callback doesn't take down the event-loop thread. Without this, a
/// panic anywhere in user code (poisoned lock, closed channel, etc.)
/// would silently kill event delivery for the rest of the session.
fn safe_invoke(label: &str, f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        log::error!("mpv {label} callback panicked; event loop continuing");
    }
}

fn event_loop(
    lib: Arc<MpvLib>,
    handle: Arc<MpvHandle>,
    shutdown: Arc<AtomicBool>,
    callbacks: Arc<MpvCallbacks>,
) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        let event = unsafe { &*lib.wait_event(handle.ptr(), 0.5) };

        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match event.event_id {
            MPV_EVENT_NONE => continue,

            MPV_EVENT_SHUTDOWN => break,

            MPV_EVENT_PROPERTY_CHANGE => {
                if event.data.is_null() {
                    continue;
                }
                let prop = unsafe { &*(event.data as *const mpv_event_property) };
                let id = event.reply_userdata;

                if prop.data.is_null() {
                    continue;
                }

                match id {
                    id if id == ObserverID::TimePos as u64 && prop.format == MPV_FORMAT_DOUBLE => {
                        let val = unsafe { *(prop.data as *const f64) };
                        if let Some(ref cb) = callbacks.on_position_change {
                            safe_invoke("on_position_change", || cb(val));
                        }
                    }
                    id if id == ObserverID::Duration as u64
                        && prop.format == MPV_FORMAT_DOUBLE =>
                    {
                        let val = unsafe { *(prop.data as *const f64) };
                        if let Some(ref cb) = callbacks.on_duration_change {
                            safe_invoke("on_duration_change", || cb(val));
                        }
                    }
                    id if id == ObserverID::PlaylistPos as u64
                        && prop.format == MPV_FORMAT_INT64 =>
                    {
                        let val = unsafe { *(prop.data as *const i64) };
                        if let Some(ref cb) = callbacks.on_playlist_pos_change {
                            safe_invoke("on_playlist_pos_change", || cb(val));
                        }
                    }
                    id if id == ObserverID::Pause as u64 && prop.format == MPV_FORMAT_FLAG => {
                        let val = unsafe { *(prop.data as *const c_int) };
                        if let Some(ref cb) = callbacks.on_pause_change {
                            safe_invoke("on_pause_change", || cb(val != 0));
                        }
                    }
                    id if id == ObserverID::IdleActive as u64
                        && prop.format == MPV_FORMAT_FLAG =>
                    {
                        let val = unsafe { *(prop.data as *const c_int) };
                        if val != 0 {
                            if let Some(ref cb) = callbacks.on_idle_active {
                                safe_invoke("on_idle_active", cb);
                            }
                        }
                    }
                    _ => {}
                }
            }

            MPV_EVENT_FILE_LOADED => {
                if let Some(ref cb) = callbacks.on_file_loaded {
                    safe_invoke("on_file_loaded", cb);
                }
            }

            MPV_EVENT_LOG_MESSAGE if !event.data.is_null() => {
                let msg = unsafe { &*(event.data as *const mpv_event_log_message) };
                if !msg.prefix.is_null() && !msg.text.is_null() {
                    let prefix = unsafe { CStr::from_ptr(msg.prefix) }
                        .to_string_lossy();
                    let text = unsafe { CStr::from_ptr(msg.text) }
                        .to_string_lossy();
                    // mpv terminates lines with `\n` itself; trim so our
                    // own logger doesn't emit blank lines. The lavf/http
                    // subsystems log full URLs at info level on connect
                    // and on transfer errors; redact_urls strips any
                    // ?X-Plex-Token= / X-Plex-Headers= query that would
                    // otherwise land in our log sinks.
                    let trimmed = text.trim_end_matches('\n');
                    if !trimmed.is_empty() {
                        let safe = redact_urls(trimmed);
                        // mpv log_level constants (from client.h):
                        // 10=FATAL, 20=ERROR, 30=WARN, 40=INFO, 50=V, 60=DEBUG, 70=TRACE.
                        match msg.log_level {
                            l if l <= 20 => log::error!("mpv[{prefix}]: {safe}"),
                            l if l <= 30 => log::warn!("mpv[{prefix}]: {safe}"),
                            l if l <= 40 => log::info!("mpv[{prefix}]: {safe}"),
                            l if l <= 60 => log::debug!("mpv[{prefix}]: {safe}"),
                            _ => log::trace!("mpv[{prefix}]: {safe}"),
                        }
                    }
                }
            }

            MPV_EVENT_END_FILE if !event.data.is_null() => {
                let ef = unsafe { &*(event.data as *const mpv_event_end_file) };
                let reason = match ef.reason {
                    MPV_END_FILE_REASON_EOF => FileEndReason::Eof,
                    MPV_END_FILE_REASON_STOP => FileEndReason::Stop,
                    MPV_END_FILE_REASON_QUIT => FileEndReason::Quit,
                    MPV_END_FILE_REASON_ERROR => {
                        let msg = unsafe {
                            CStr::from_ptr(lib.error_string(ef.error))
                                .to_string_lossy()
                                .into_owned()
                        };
                        FileEndReason::Error(msg)
                    }
                    MPV_END_FILE_REASON_REDIRECT => FileEndReason::Redirect,
                    _ => FileEndReason::Unknown,
                };
                if let Some(ref cb) = callbacks.on_file_ended {
                    safe_invoke("on_file_ended", || cb(reason));
                }
            }

            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mpv_version_at_least;

    #[test]
    fn parses_release_versions() {
        assert!(!mpv_version_at_least("mpv 0.35.1", 0, 38));
        assert!(!mpv_version_at_least("mpv 0.37.0", 0, 38));
        assert!(mpv_version_at_least("mpv 0.38.0", 0, 38));
        assert!(mpv_version_at_least("mpv 0.39.0", 0, 38));
        assert!(mpv_version_at_least("mpv 1.0.0", 0, 38));
    }

    #[test]
    fn parses_homebrew_v_prefix() {
        // Homebrew's libmpv on macOS reports the version with a leading
        // `v` (e.g. "mpv v0.41.0"). Apt does not. Original parser missed
        // this and returned false on perfectly modern mpv, silently
        // falling back to the pre-0.38 loadfile signature.
        assert!(mpv_version_at_least("mpv v0.41.0", 0, 38));
        assert!(mpv_version_at_least("mpv v0.38.0", 0, 38));
        assert!(!mpv_version_at_least("mpv v0.37.0", 0, 38));
    }

    #[test]
    fn parses_versions_with_trailing_suffix() {
        // Distro patch suffix (Debian/Ubuntu style)
        assert!(mpv_version_at_least("mpv 0.40.0-1", 0, 38));
        // Pre-release tag
        assert!(mpv_version_at_least("mpv 0.38.0-rc1", 0, 38));
    }

    #[test]
    fn rejects_unparseable_versions() {
        assert!(!mpv_version_at_least("mpv git-deadbeef", 0, 38));
        assert!(!mpv_version_at_least("", 0, 38));
        assert!(!mpv_version_at_least("0.38.0", 0, 38)); // missing "mpv " prefix
        assert!(!mpv_version_at_least("mpv 0", 0, 38)); // no minor
    }
}
