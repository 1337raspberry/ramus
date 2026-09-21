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
use ramus_core::playback::spectrum_tap::{TapConfig, TapFeed, TapFrame, TapLineParser};
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
    /// Log level requested at init (`RAMUS_MPV_LOG_LEVEL` or `info`).
    /// `set_verbose_log` raises to `v` above it while the spectrum tap is
    /// installed and restores it afterwards.
    base_log_level: String,
    _event_thread: Option<thread::JoinHandle<()>>,
}

/// mpv log level names in increasing verbosity, as accepted by
/// `mpv_request_log_messages`.
const MPV_LOG_LEVELS: [&str; 8] = ["no", "fatal", "error", "warn", "info", "v", "debug", "trace"];

fn log_level_rank(level: &str) -> usize {
    MPV_LOG_LEVELS
        .iter()
        .position(|l| *l == level)
        .unwrap_or(4)
}

/// Upper bound on frames held back before a batch is handed to the
/// callback regardless of what event follows. Bursts are normally ~5
/// frames and are flushed by the next non-log event (a position tick or
/// the wait timeout); this only bites on unusually long bursts such as
/// the catch-up after a seek.
const TAP_FLUSH_BATCH: usize = 16;

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
            let base_log_level = std::env::var("RAMUS_MPV_LOG_LEVEL")
                .unwrap_or_else(|_| "info".into());
            let level = CString::new(base_log_level.clone()).unwrap();
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

            // Read back the cache / demuxer caps so we can verify our
            // `set_option_string` calls actually landed. mpv silently
            // discards unknown options and doesn't reject out-of-range
            // values; without a read-back the only way to detect "option
            // didn't apply" is via downstream symptoms (e.g. recorder
            // failing because the demuxer slurped a whole track).
            for opt in [
                "demuxer-max-bytes",
                "demuxer-readahead-secs",
                "cache-secs",
                "cache",
            ] {
                if let Some(v) = read_string_property(&lib, ctx, opt) {
                    log::info!("libmpv: {opt}={v}");
                } else {
                    log::warn!("libmpv: could not read {opt}");
                }
            }

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
                base_log_level,
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
/// not "fail safe on new mpv" — getting it wrong on a new mpv breaks every
/// per-file option, such as the `start=` resume (loadfile would put it in
/// the index slot).
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
        let rc = unsafe {
            let name = CString::new("af").unwrap();
            self.lib
                .set_property_string(self.handle.ptr(), name.as_ptr(), val.as_ptr())
        };
        if rc < 0 {
            // A rejected chain leaves the previous one running, so a missing
            // EQ or a blank visualiser is otherwise silent. The chain string
            // carries no URLs or tokens.
            let reason = unsafe { CStr::from_ptr(self.lib.error_string(rc)) }.to_string_lossy();
            log::error!("mpv rejected af chain ({reason}): {value}");
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

    fn set_verbose_log(&self, enabled: bool) {
        // Only ever raise: a `RAMUS_MPV_LOG_LEVEL=debug` session keeps its
        // debug stream through tap toggles.
        let level = if enabled && log_level_rank(&self.base_log_level) < log_level_rank("v") {
            "v"
        } else {
            self.base_log_level.as_str()
        };
        let c = CString::new(level).unwrap();
        unsafe {
            self.lib.request_log_messages(self.handle.ptr(), c.as_ptr());
        }
        log::debug!("mpv log level -> {level}");
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

/// Hand the collected tap frames to the callback and empty the buffer.
fn flush_tap_frames(callbacks: &MpvCallbacks, frames: &mut Vec<TapFrame>) {
    if frames.is_empty() {
        return;
    }
    let batch = std::mem::take(frames);
    if let Some(ref cb) = callbacks.on_spectrum_frames {
        safe_invoke("on_spectrum_frames", || cb(batch));
    }
}

fn event_loop(
    lib: Arc<MpvLib>,
    handle: Arc<MpvHandle>,
    shutdown: Arc<AtomicBool>,
    callbacks: Arc<MpvCallbacks>,
) {
    // Live spectrum tap transport. The tap's `ashowinfo` printer writes
    // one line per frame into FFmpeg's log, which mpv forwards here as an
    // `ffmpeg`-prefixed message at level `v` (mpv buffers the printer's
    // partial writes into whole lines). The parser decodes it, and it is
    // swallowed before the generic log forwarding below.
    let mut tap_parser = TapLineParser::new(TapConfig::default().normalised().bands);
    let mut tap_frames: Vec<TapFrame> = Vec::new();

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }

        let event = unsafe { &*lib.wait_event(handle.ptr(), 0.5) };

        if shutdown.load(Ordering::Acquire) {
            break;
        }

        // mpv runs the filter chain when the audio output needs data, so
        // frames arrive in bursts. Anything that is not another log line
        // (a position tick, the wait timeout) marks the end of a burst.
        if event.event_id != MPV_EVENT_LOG_MESSAGE {
            flush_tap_frames(&callbacks, &mut tap_frames);
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
                    // The tap parser needs the untrimmed text: the newline
                    // is how it tells a complete line from a fragment.
                    if prefix.starts_with("ffmpeg") {
                        match tap_parser.feed(&text) {
                            TapFeed::Frame(frame) => {
                                tap_frames.push(frame);
                                if tap_frames.len() >= TAP_FLUSH_BATCH {
                                    flush_tap_frames(&callbacks, &mut tap_frames);
                                }
                                continue;
                            }
                            TapFeed::Consumed => {
                                // A complete tap line that did not decode
                                // is worth seeing when chasing frame loss.
                                if text.ends_with('\n') {
                                    log::trace!(
                                        "spectrum tap: undecodable line ({} bytes): {}",
                                        text.len(),
                                        text.trim_end()
                                    );
                                }
                                continue;
                            }
                            TapFeed::Ignored => {}
                        }
                    }
                    let trimmed = text.trim_end_matches('\n');
                    if !trimmed.is_empty() {
                        let safe = redact_urls(trimmed);
                        // A graph that fails to configure makes mpv drop
                        // that `af` entry and carry on without it, so a
                        // missing EQ or a blank visualiser is otherwise
                        // silent. Call it out explicitly.
                        if safe.contains("failed to configure the filter graph")
                            || safe.contains("Disabling filter")
                        {
                            log::warn!(
                                "mpv af chain rejected — the EQ or the spectrum tap is absent: {safe}"
                            );
                        }
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

/// Live probes of the spectrum tap against the libmpv on this machine.
///
/// Ignored by default: they dlopen libmpv, open the audio output (muted)
/// and play audio. Run them on any platform whose libmpv build is in
/// doubt (the Linux AppImage's FFmpeg 4.4 in particular):
///
/// ```text
/// RUST_LOG=info cargo test -p ramus-tauri --lib -- --ignored tap_probe --nocapture
/// ```
///
/// `tap_probe_produces_frames_from_a_tone` proves the graph parses on
/// that FFmpeg, `join`'s channel layout is accepted, both banks'
/// `ashowinfo` lines reach the client log at level `v` and pair up into
/// frames of the expected shape, the checksums decode to levels, the tone
/// lands in the right band on both channels, frames lead `time-pos`, and
/// removing the tap stops the flow.
///
/// `tap_probe_cost` plays a local file through the controller with and
/// without the tap and prints this process's CPU share for each:
///
/// ```text
/// RAMUS_TAP_PROBE_TRACK=/path/to/track.flac RAMUS_TAP_PROBE_SECS=20 \
///   RUST_LOG=info cargo test --release -p ramus-tauri --lib -- --ignored tap_probe_cost --nocapture
/// ```
#[cfg(test)]
mod tap_probe {
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;

    use ramus_core::playback::player::build_af_string;
    use ramus_core::playback::spectrum_tap::{band_frequencies, TapConfig, TapFrame};

    use super::*;

    struct Harness {
        mpv: MpvController,
        frames: Arc<Mutex<Vec<TapFrame>>>,
        positions: Arc<Mutex<Vec<f64>>>,
    }

    fn harness() -> Harness {
        let _ = env_logger::builder().is_test(true).try_init();
        let lib = Arc::new(MpvLib::load().expect("libmpv must be loadable"));
        let frames: Arc<Mutex<Vec<TapFrame>>> = Arc::new(Mutex::new(Vec::new()));
        let positions: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let pos_sink = positions.clone();
        let callbacks = Arc::new(MpvCallbacks {
            on_spectrum_frames: Some(Box::new(move |batch| sink.lock().extend(batch))),
            on_position_change: Some(Box::new(move |p| pos_sink.lock().push(p))),
            ..Default::default()
        });
        let mpv = MpvController::new(lib, callbacks).expect("mpv controller");
        mpv.set_volume(0.0);
        Harness {
            mpv,
            frames,
            positions,
        }
    }

    /// Install the tap the way `AudioPlayer::set_spectrum_tap` does: raise
    /// the log level, then compose EQ + tap.
    fn install_tap(mpv: &MpvController) -> TapConfig {
        let cfg = TapConfig::default().normalised();
        mpv.set_verbose_log(true);
        mpv.set_audio_filters(&build_af_string(true, &[0.0; 10], Some(&cfg)));
        cfg
    }

    fn remove_tap(mpv: &MpvController) {
        mpv.set_audio_filters(&build_af_string(true, &[0.0; 10], None));
        mpv.set_verbose_log(false);
    }

    fn wait_for_frames(frames: &Mutex<Vec<TapFrame>>, want: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && frames.lock().len() < want {
            std::thread::sleep(Duration::from_millis(50));
        }
        frames.lock().len()
    }

    #[test]
    #[ignore]
    fn tap_probe_produces_frames_from_a_tone() {
        let h = harness();
        let cfg = install_tap(&h.mpv);
        h.mpv.load_file(
            "av://lavfi:sine=frequency=440:sample_rate=44100:duration=4",
            LoadMode::Replace,
            None,
        );

        let want = 2 * cfg.fps as usize;
        let n = wait_for_frames(&h.frames, want, Duration::from_secs(10));
        assert!(n >= want, "expected at least {want} frames within 10 s, got {n}");
        let got = h.frames.lock().clone();

        // Shape: every band of both channels.
        assert!(got.iter().all(|f| f.db.len() == cfg.frame_width()));
        // Monotonic pts, one frame period apart.
        let period = 1.0 / cfg.fps as f64;
        for w in got.windows(2) {
            let dt = w[1].pts - w[0].pts;
            assert!(
                (dt - period).abs() < period * 0.25,
                "frame spacing {dt:.4}s, expected {period:.4}s"
            );
        }
        // The mono tone is duplicated into both channels, so each half
        // peaks in the band nearest 440 Hz.
        let freqs = band_frequencies(&cfg);
        let mid = &got[got.len() / 2];
        let mut loudest = 0;
        let mut f = 0.0;
        for (channel, half) in mid.db.chunks(cfg.bands).enumerate() {
            loudest = (0..cfg.bands)
                .max_by(|&a, &b| half[a].partial_cmp(&half[b]).unwrap())
                .unwrap();
            f = freqs[loudest];
            assert!(
                ((f - 440.0) / 440.0).abs() < 0.15,
                "channel {channel}: loudest band {loudest} at {f:.0} Hz, expected ~440 Hz; {half:?}"
            );
            // Levels are sane: the tone's band is well above the floor
            // and a far-away band is well below it.
            assert!(half[loudest] > -30.0, "channel {channel}: tone band {} dB", half[loudest]);
            assert!(half[cfg.bands - 1] < half[loudest] - 20.0, "channel {channel}: {half:?}");
        }
        // Both halves carry the same signal, so they agree closely.
        let (l, r) = mid.db.split_at(cfg.bands);
        assert!(
            l.iter().zip(r).all(|(a, b)| (a - b).abs() < 1.0),
            "left/right disagree: {l:?} vs {r:?}"
        );
        // Frames lead (or at worst match) the reported position.
        let last_pos = h.positions.lock().last().copied().unwrap_or(0.0);
        let last_pts = got.last().unwrap().pts;
        assert!(
            last_pts + 0.1 >= last_pos,
            "frames should lead time-pos: last pts {last_pts:.3} vs pos {last_pos:.3}"
        );
        log::info!(
            "tap_probe: {} frames, loudest band {loudest} ({f:.0} Hz) at {:.1} dB, lead {:.3}s",
            got.len(),
            mid.db[loudest],
            last_pts - last_pos
        );

        // Removing the tap stops the flow.
        remove_tap(&h.mpv);
        std::thread::sleep(Duration::from_millis(400));
        let settled = h.frames.lock().len();
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(h.frames.lock().len(), settled, "frames kept arriving after removal");
    }

    /// Cumulative CPU time of this process in seconds, via `ps` (portable
    /// across the Unix desktops without a libc dependency). `None` where
    /// `ps` is unavailable.
    fn process_cpu_seconds() -> Option<f64> {
        let out = std::process::Command::new("ps")
            .args(["-o", "cputime=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        // `M:SS.ss` (macOS) or `HH:MM:SS` (Linux): fold the fields as base 60.
        text.trim()
            .split(':')
            .try_fold(0.0, |acc, part| part.trim().parse::<f64>().ok().map(|v| acc * 60.0 + v))
    }

    #[test]
    #[ignore]
    fn tap_probe_cost() {
        let Ok(track) = std::env::var("RAMUS_TAP_PROBE_TRACK") else {
            eprintln!("set RAMUS_TAP_PROBE_TRACK to a local audio file");
            return;
        };
        let secs: f64 = std::env::var("RAMUS_TAP_PROBE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20.0);
        let h = harness();
        let mut report = Vec::new();
        for (label, with_tap) in [("no tap", false), ("tap", true)] {
            h.frames.lock().clear();
            h.positions.lock().clear();
            let cfg = with_tap.then(|| install_tap(&h.mpv));
            h.mpv
                .load_file(&track, LoadMode::Replace, Some("start=30"));
            // Let the pipeline settle before the window opens.
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && h.positions.lock().len() < 3 {
                std::thread::sleep(Duration::from_millis(50));
            }
            let cpu0 = process_cpu_seconds();
            let t0 = Instant::now();
            std::thread::sleep(Duration::from_secs_f64(secs));
            let wall = t0.elapsed().as_secs_f64();
            let cpu = match (cpu0, process_cpu_seconds()) {
                (Some(a), Some(b)) => Some(b - a),
                _ => None,
            };
            let frames = h.frames.lock().len();
            if let Some(cfg) = cfg {
                assert!(
                    frames as f64 > secs * cfg.fps as f64 * 0.8,
                    "{label}: only {frames} frames in {wall:.1}s"
                );
                remove_tap(&h.mpv);
            }
            h.mpv.stop();
            std::thread::sleep(Duration::from_millis(300));
            let share = cpu.map(|c| format!("{:.1}% of one core", c / wall * 100.0));
            report.push(format!(
                "{label:>6}: {} ({frames} frames in {wall:.1}s)",
                share.unwrap_or_else(|| "cpu unavailable".into())
            ));
        }
        for line in &report {
            println!("tap_probe_cost {line}");
            log::info!("tap_probe_cost {line}");
        }
    }

    /// The frontend can issue install/remove requests back to back (a
    /// remount is remove-then-install with no gap) and each command runs
    /// on its own task, so their mpv calls can interleave. This probe pins
    /// down what mpv does under each interleaving, and whether a clean
    /// remove/install afterwards restores the flow:
    ///
    /// ```text
    /// RUST_LOG=info cargo test -p ramus-tauri --lib -- --ignored tap_probe_recovers --nocapture
    /// ```
    #[test]
    #[ignore]
    fn tap_probe_recovers_after_interleaved_toggles() {
        let h = harness();
        h.mpv.load_file(
            "av://lavfi:sine=frequency=440:sample_rate=44100:duration=180",
            LoadMode::Replace,
            None,
        );
        let cfg = install_tap(&h.mpv);
        let want = cfg.fps as usize;
        let n = wait_for_frames(&h.frames, want, Duration::from_secs(10));
        assert!(n >= want, "baseline: {n} frames");

        let flows = |label: &str| -> bool {
            h.frames.lock().clear();
            let n = wait_for_frames(&h.frames, want, Duration::from_secs(4));
            println!("tap_probe_recovers {label}: {n} frames in <=4s");
            n >= want
        };

        let tap = build_af_string(true, &[0.0; 10], Some(&cfg));
        let no_tap = build_af_string(true, &[0.0; 10], None);

        // A: the remove's calls land last (chain gone, level lowered).
        h.mpv.set_verbose_log(true);
        h.mpv.set_audio_filters(&tap);
        h.mpv.set_audio_filters(&no_tap);
        h.mpv.set_verbose_log(false);
        let _ = flows("after A (chain gone, level low)");
        remove_tap(&h.mpv);
        install_tap(&h.mpv);
        assert!(flows("clean cycle after A"));

        // B: chain kept, level lowered last.
        h.mpv.set_verbose_log(true);
        h.mpv.set_audio_filters(&tap);
        h.mpv.set_verbose_log(false);
        let _ = flows("after B (chain kept, level low)");
        remove_tap(&h.mpv);
        install_tap(&h.mpv);
        assert!(flows("clean cycle after B"));

        // C: level raised last, chain removed last.
        h.mpv.set_verbose_log(true);
        h.mpv.set_audio_filters(&tap);
        h.mpv.set_audio_filters(&no_tap);
        let _ = flows("after C (chain gone, level high)");
        remove_tap(&h.mpv);
        install_tap(&h.mpv);
        assert!(flows("clean cycle after C"));

        // Concurrent install/remove/install, as a remount issues them.
        for i in 0..8 {
            let m = &h.mpv;
            std::thread::scope(|s| {
                s.spawn(|| {
                    m.set_verbose_log(true);
                    m.set_audio_filters(&tap);
                });
                s.spawn(|| {
                    m.set_audio_filters(&no_tap);
                    m.set_verbose_log(false);
                });
                s.spawn(|| {
                    m.set_verbose_log(true);
                    m.set_audio_filters(&tap);
                });
            });
            let _ = flows(&format!("after concurrent triple {i}"));
            remove_tap(&h.mpv);
            install_tap(&h.mpv);
            assert!(flows(&format!("clean cycle after concurrent triple {i}")));
        }
        remove_tap(&h.mpv);
        h.mpv.stop();
    }
}
