//! Raw libmpv C FFI bindings — minimal set needed for audio-only playback.
//!
//! libmpv is loaded at runtime via `libloading` rather than statically linked.
//! This means the app compiles on any platform without libmpv headers or
//! import libs, and the actual `.dylib` / `.so` / `.dll` is looked up when
//! the first `MpvController` is constructed. See `MpvLib::load()` for the
//! path search strategy.

#![allow(non_camel_case_types, dead_code)]

use libloading::{Library, Symbol};
use std::os::raw::{c_char, c_int, c_void};

// --- Opaque handle + POD types (unchanged from the static-linking version) ---

pub enum mpv_handle {}

// Formats
pub const MPV_FORMAT_NONE: c_int = 0;
pub const MPV_FORMAT_STRING: c_int = 1;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;
pub const MPV_FORMAT_NODE: c_int = 6;
pub const MPV_FORMAT_NODE_ARRAY: c_int = 7;
pub const MPV_FORMAT_NODE_MAP: c_int = 8;
pub const MPV_FORMAT_BYTE_ARRAY: c_int = 9;

// Event IDs
pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

// End-file reasons
pub const MPV_END_FILE_REASON_EOF: c_int = 0;
pub const MPV_END_FILE_REASON_STOP: c_int = 2;
pub const MPV_END_FILE_REASON_QUIT: c_int = 3;
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;
pub const MPV_END_FILE_REASON_REDIRECT: c_int = 5;

#[repr(C)]
pub struct mpv_event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
}

// --- Node format (for complex properties like af-metadata/<label>) ---
//
// mpv returns `af-metadata/astats` as an MPV_FORMAT_NODE whose inner node is
// MPV_FORMAT_NODE_MAP: a list of (key string, value node) pairs where every
// value node is itself an MPV_FORMAT_STRING. We only read string values out
// of this tree, so other union variants (flag, int64, double, byte_array)
// are declared for correct struct layout but unused.

#[repr(C)]
pub union mpv_node_u {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: f64,
    pub list: *mut mpv_node_list,
    pub ba: *mut c_void, // byte_array — unused
}

#[repr(C)]
pub struct mpv_node {
    pub u: mpv_node_u,
    pub format: c_int,
}

#[repr(C)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

// --- Function pointer types ---
//
// One type alias per symbol, matching the signatures libmpv exposes in
// `client.h`. These stay unsafe because we're still passing raw pointers
// into C code; `libloading` only takes care of resolving the symbol.

type FnCreate = unsafe extern "C" fn() -> *mut mpv_handle;
type FnInitialize = unsafe extern "C" fn(*mut mpv_handle) -> c_int;
type FnDestroy = unsafe extern "C" fn(*mut mpv_handle);
type FnSetOptionString =
    unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnCommand = unsafe extern "C" fn(*mut mpv_handle, *mut *const c_char) -> c_int;
type FnSetProperty =
    unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnSetPropertyString =
    unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnGetProperty =
    unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnObserveProperty =
    unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, c_int) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut mpv_handle, f64) -> *mut mpv_event;
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;

/// Runtime-loaded libmpv.
///
/// Holds the owning `Library` plus a cached function pointer for each symbol
/// we call. **Field declaration order matters**: Rust drops struct fields in
/// declaration order, so `_lib` is declared LAST to guarantee it is dropped
/// last — if the library were unloaded before the function pointers, any
/// in-flight call would jump into freed memory.
pub struct MpvLib {
    create: FnCreate,
    initialize: FnInitialize,
    destroy: FnDestroy,
    set_option_string: FnSetOptionString,
    command: FnCommand,
    set_property: FnSetProperty,
    set_property_string: FnSetPropertyString,
    get_property: FnGetProperty,
    observe_property: FnObserveProperty,
    wait_event: FnWaitEvent,
    error_string: FnErrorString,
    _lib: Library,
}

// SAFETY: libmpv is explicitly thread-safe (that's the whole point of the
// `mpv_wait_event` / property observer design). The raw function pointers
// stored in `MpvLib` are plain integers with no interior mutability, and
// the `Library` handle itself is `Send + Sync` on all supported platforms.
unsafe impl Send for MpvLib {}
unsafe impl Sync for MpvLib {}

// Each wrapper below is a one-line trampoline into libmpv, so documenting
// the safety contract per-method would just be 11 copies of "caller must
// pass a valid ctx pointer, valid C strings, and a buffer matching the
// format tag". The real contract is whatever libmpv documents in
// `client.h` — the wrappers add nothing on top.
#[allow(clippy::missing_safety_doc)]
impl MpvLib {
    /// Attempt to locate and load libmpv.
    ///
    /// Returns a detailed error listing every path tried if loading fails,
    /// so users get a clear "here's where I looked, install mpv there"
    /// message instead of a cryptic dlopen error.
    pub fn load() -> Result<Self, String> {
        let lib = open_library()?;
        unsafe {
            let create: Symbol<FnCreate> = resolve(&lib, b"mpv_create\0")?;
            let initialize: Symbol<FnInitialize> = resolve(&lib, b"mpv_initialize\0")?;
            let destroy: Symbol<FnDestroy> = resolve(&lib, b"mpv_destroy\0")?;
            let set_option_string: Symbol<FnSetOptionString> =
                resolve(&lib, b"mpv_set_option_string\0")?;
            let command: Symbol<FnCommand> = resolve(&lib, b"mpv_command\0")?;
            let set_property: Symbol<FnSetProperty> = resolve(&lib, b"mpv_set_property\0")?;
            let set_property_string: Symbol<FnSetPropertyString> =
                resolve(&lib, b"mpv_set_property_string\0")?;
            let get_property: Symbol<FnGetProperty> = resolve(&lib, b"mpv_get_property\0")?;
            let observe_property: Symbol<FnObserveProperty> =
                resolve(&lib, b"mpv_observe_property\0")?;
            let wait_event: Symbol<FnWaitEvent> = resolve(&lib, b"mpv_wait_event\0")?;
            let error_string: Symbol<FnErrorString> = resolve(&lib, b"mpv_error_string\0")?;

            // Copy each fn pointer out of its Symbol<'_>. The pointers remain
            // valid as long as `_lib` is alive, which is guaranteed by the
            // struct field drop order (see the struct doc comment above).
            Ok(Self {
                create: *create,
                initialize: *initialize,
                destroy: *destroy,
                set_option_string: *set_option_string,
                command: *command,
                set_property: *set_property,
                set_property_string: *set_property_string,
                get_property: *get_property,
                observe_property: *observe_property,
                wait_event: *wait_event,
                error_string: *error_string,
                _lib: lib,
            })
        }
    }

    // --- Thin wrappers around each symbol. Still `unsafe` because the
    // caller is still responsible for valid pointers / lifetimes. ---

    #[inline]
    pub unsafe fn create(&self) -> *mut mpv_handle {
        (self.create)()
    }

    #[inline]
    pub unsafe fn initialize(&self, ctx: *mut mpv_handle) -> c_int {
        (self.initialize)(ctx)
    }

    #[inline]
    pub unsafe fn destroy(&self, ctx: *mut mpv_handle) {
        (self.destroy)(ctx)
    }

    #[inline]
    pub unsafe fn set_option_string(
        &self,
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int {
        (self.set_option_string)(ctx, name, data)
    }

    #[inline]
    pub unsafe fn command(&self, ctx: *mut mpv_handle, args: *mut *const c_char) -> c_int {
        (self.command)(ctx, args)
    }

    #[inline]
    pub unsafe fn set_property(
        &self,
        ctx: *mut mpv_handle,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> c_int {
        (self.set_property)(ctx, name, format, data)
    }

    #[inline]
    pub unsafe fn set_property_string(
        &self,
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int {
        (self.set_property_string)(ctx, name, data)
    }

    #[inline]
    pub unsafe fn get_property(
        &self,
        ctx: *mut mpv_handle,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> c_int {
        (self.get_property)(ctx, name, format, data)
    }

    #[inline]
    pub unsafe fn observe_property(
        &self,
        ctx: *mut mpv_handle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int {
        (self.observe_property)(ctx, reply_userdata, name, format)
    }

    #[inline]
    pub unsafe fn wait_event(&self, ctx: *mut mpv_handle, timeout: f64) -> *mut mpv_event {
        (self.wait_event)(ctx, timeout)
    }

    #[inline]
    pub unsafe fn error_string(&self, error: c_int) -> *const c_char {
        (self.error_string)(error)
    }
}

unsafe fn resolve<'a, T>(lib: &'a Library, name: &[u8]) -> Result<Symbol<'a, T>, String> {
    lib.get(name).map_err(|e| {
        let sym = std::str::from_utf8(name.strip_suffix(b"\0").unwrap_or(name)).unwrap_or("?");
        format!("libmpv missing symbol `{sym}`: {e}")
    })
}

// --- Library search strategy ---
//
// Order of preference:
//   1. MPV_LIB_PATH env var — explicit dev override
//   2. Next to the current executable (bundled distribution)
//   3. macOS .app bundle Frameworks/ dir (relative to executable)
//   4. Platform-specific system paths where Homebrew / apt usually install it
//   5. Bare filename — lets the OS dynamic linker search DYLD/LD/PATH
//
// Each candidate is attempted with `Library::new`. The first one that
// succeeds wins; all failures are collected into the returned error so
// the user can see exactly where we looked.

fn open_library() -> Result<Library, String> {
    let candidates = candidate_paths();
    let mut errors = Vec::with_capacity(candidates.len());
    for path in &candidates {
        match unsafe { Library::new(path) } {
            Ok(lib) => {
                log::info!("libmpv: loaded from {path}");
                return Ok(lib);
            }
            Err(e) => errors.push(format!("  {path}: {e}")),
        }
    }
    Err(format!(
        "Could not load libmpv. Tried:\n{}\n\n\
         Install libmpv:\n\
         \x20 macOS:   brew install mpv\n\
         \x20 Linux:   apt install libmpv2   (or libmpv1 on older distros)\n\
         \x20 Windows: libmpv-2.dll should ship alongside the .exe — if you see this,\n\
         \x20          either the installer is incomplete or you're running a dev build\n\
         \x20          without the DLL on PATH",
        errors.join("\n")
    ))
}

fn candidate_paths() -> Vec<String> {
    let mut paths = Vec::new();

    if let Ok(env_path) = std::env::var("MPV_LIB_PATH") {
        paths.push(env_path);
    }

    let lib_name = default_lib_name();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled next to the .exe (Windows installer, Linux AppImage root)
            paths.push(dir.join(lib_name).to_string_lossy().into_owned());
            // Tauri places bundle.resources here at runtime
            paths.push(
                dir.join("resources")
                    .join(lib_name)
                    .to_string_lossy()
                    .into_owned(),
            );
            // macOS .app bundle: dir is Contents/MacOS, we want Contents/Frameworks
            #[cfg(target_os = "macos")]
            if let Some(contents) = dir.parent() {
                paths.push(
                    contents
                        .join("Frameworks")
                        .join(lib_name)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        paths.push("/opt/homebrew/lib/libmpv.2.dylib".into()); // Apple Silicon brew
        paths.push("/opt/homebrew/lib/libmpv.dylib".into());
        paths.push("/usr/local/lib/libmpv.2.dylib".into()); // Intel brew
        paths.push("/usr/local/lib/libmpv.dylib".into());
        paths.push("libmpv.2.dylib".into());
        paths.push("libmpv.dylib".into());
    }

    #[cfg(target_os = "linux")]
    {
        paths.push("libmpv.so.2".into());
        paths.push("libmpv.so.1".into());
        paths.push("libmpv.so".into());
    }

    #[cfg(target_os = "windows")]
    {
        paths.push("libmpv-2.dll".into());
        paths.push("mpv-2.dll".into());
    }

    paths
}

const fn default_lib_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "libmpv-2.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libmpv.2.dylib"
    }
    #[cfg(target_os = "linux")]
    {
        "libmpv.so.2"
    }
}
