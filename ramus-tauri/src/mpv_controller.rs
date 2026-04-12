//! Real libmpv controller implementing the MpvPlayer trait.
//!
//! Creates an mpv instance for audio-only playback, runs an event loop
//! on a background thread, and dispatches callbacks to the caller.

use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use ramus_core::playback::mpv::{FileEndReason, LoadMode, MpvCallbacks, MpvPlayer, ObserverID};

use crate::mpv_ffi::*;

// --- Thread-safe wrapper for the raw mpv handle ---

struct MpvHandle(*mut mpv_handle);
unsafe impl Send for MpvHandle {}
unsafe impl Sync for MpvHandle {}

impl MpvHandle {
    fn ptr(&self) -> *mut mpv_handle {
        self.0
    }
}

// --- MpvController ---

pub struct MpvController {
    lib: Arc<MpvLib>,
    handle: Arc<MpvHandle>,
    shutdown: Arc<AtomicBool>,
    _event_thread: Option<thread::JoinHandle<()>>,
}

impl MpvController {
    /// Create and initialize a new mpv instance with a background event
    /// loop thread that dispatches `callbacks`.
    ///
    /// `lib` is the runtime-loaded libmpv — load it once at app startup
    /// (via `MpvLib::load()`) and share the `Arc` across controllers.
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

            // Apply audio-only options
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

            // Register property observers
            let props = ramus_core::playback::mpv::observed_properties();
            for (name, id) in &props {
                let n = CString::new(*name).unwrap();
                let fmt = match id {
                    ObserverID::TimePos | ObserverID::Duration | ObserverID::CacheSpeed => {
                        MPV_FORMAT_DOUBLE
                    }
                    ObserverID::Pause | ObserverID::PausedForCache | ObserverID::IdleActive => {
                        MPV_FORMAT_FLAG
                    }
                    ObserverID::PlaylistPos | ObserverID::CacheBufferingState => MPV_FORMAT_INT64,
                };
                lib.observe_property(ctx, *id as u64, n.as_ptr(), fmt);
            }

            // Default volume (100 = unity gain, no attenuation)
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

            // Spawn background event loop
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
                _event_thread: Some(event_thread),
            })
        }
    }

    fn command(&self, args: &[&str]) {
        unsafe {
            let c_args: Vec<CString> = args.iter().map(|s| CString::new(*s).unwrap()).collect();
            let mut ptrs: Vec<*const i8> = c_args.iter().map(|s| s.as_ptr()).collect();
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

    fn get_property_double(&self, name: &str) -> f64 {
        unsafe {
            let n = CString::new(name).unwrap();
            let mut v: f64 = 0.0;
            self.lib.get_property(
                self.handle.ptr(),
                n.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            );
            v
        }
    }
}

impl MpvPlayer for MpvController {
    fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>) {
        // mpv's `loadfile` command: `loadfile <url> <flags> [<index>] [<options>]`.
        // For replace/append/append-play, <index> is unused — options goes in slot 4
        // (we pass "-1" in the index slot as the libmpv accepted "no index" sentinel).
        match options {
            Some(opts) => self.command(&["loadfile", url, mode.as_str(), "-1", opts]),
            None => self.command(&["loadfile", url, mode.as_str()]),
        }
    }

    fn load_file_at(&self, url: &str, index: i64, options: Option<&str>) {
        let idx = index.to_string();
        match options {
            Some(opts) => self.command(&["loadfile", url, "insert-at", &idx, opts]),
            None => self.command(&["loadfile", url, "insert-at", &idx]),
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
        self.get_property_double("volume")
    }

    fn set_audio_filters(&self, value: &str) {
        unsafe {
            let name = CString::new("af").unwrap();
            let val = CString::new(value).unwrap();
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
}

impl Drop for MpvController {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock mpv_wait_event so the event loop thread exits
        self.command(&["quit"]);
        if let Some(t) = self._event_thread.take() {
            let _ = t.join();
        }
        unsafe {
            self.lib.destroy(self.handle.ptr());
        }
    }
}

// --- Event loop (runs on background thread) ---

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
                    id if id == ObserverID::TimePos as u64 => {
                        if prop.format == MPV_FORMAT_DOUBLE {
                            let val = unsafe { *(prop.data as *const f64) };
                            if let Some(ref cb) = callbacks.on_position_change {
                                cb(val);
                            }
                        }
                    }
                    id if id == ObserverID::Duration as u64 => {
                        if prop.format == MPV_FORMAT_DOUBLE {
                            let val = unsafe { *(prop.data as *const f64) };
                            if let Some(ref cb) = callbacks.on_duration_change {
                                cb(val);
                            }
                        }
                    }
                    id if id == ObserverID::PlaylistPos as u64 => {
                        if prop.format == MPV_FORMAT_INT64 {
                            let val = unsafe { *(prop.data as *const i64) };
                            if let Some(ref cb) = callbacks.on_playlist_pos_change {
                                cb(val);
                            }
                        }
                    }
                    id if id == ObserverID::Pause as u64 => {
                        if prop.format == MPV_FORMAT_FLAG {
                            let val = unsafe { *(prop.data as *const c_int) };
                            if let Some(ref cb) = callbacks.on_pause_change {
                                cb(val != 0);
                            }
                        }
                    }
                    id if id == ObserverID::PausedForCache as u64 => {
                        if prop.format == MPV_FORMAT_FLAG {
                            let val = unsafe { *(prop.data as *const c_int) };
                            if let Some(ref cb) = callbacks.on_buffering_change {
                                cb(val != 0);
                            }
                        }
                    }
                    id if id == ObserverID::IdleActive as u64 => {
                        if prop.format == MPV_FORMAT_FLAG {
                            let val = unsafe { *(prop.data as *const c_int) };
                            if val != 0 {
                                if let Some(ref cb) = callbacks.on_idle_active {
                                    cb();
                                }
                            }
                        }
                    }
                    id if id == ObserverID::CacheBufferingState as u64 => {
                        if prop.format == MPV_FORMAT_INT64 {
                            let val = unsafe { *(prop.data as *const i64) };
                            if let Some(ref cb) = callbacks.on_cache_state_change {
                                cb(val);
                            }
                        }
                    }
                    id if id == ObserverID::CacheSpeed as u64 => {
                        if prop.format == MPV_FORMAT_DOUBLE {
                            let val = unsafe { *(prop.data as *const f64) };
                            if let Some(ref cb) = callbacks.on_cache_speed_change {
                                cb(val);
                            }
                        }
                    }
                    _ => {}
                }
            }

            MPV_EVENT_FILE_LOADED => {
                if let Some(ref cb) = callbacks.on_file_loaded {
                    cb();
                }
            }

            MPV_EVENT_END_FILE => {
                if !event.data.is_null() {
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
                        cb(reason);
                    }
                }
            }

            _ => {}
        }
    }
}

