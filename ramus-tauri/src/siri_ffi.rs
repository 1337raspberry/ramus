//! C ABI shims for the platform voice-assistant / App Intents integration.
//!
//! The native intent layer (compiled into the app target) calls these to answer
//! natural-language library questions. Kept intentionally tiny — all real work
//! lives in [`ramus_core::siri_probe`], which opens its own read-only view of the
//! on-disk cache so an intent can run without the app's live state. Declared in
//! the Swift bridging header as plain C.

use std::ffi::{c_char, CStr, CString};

/// Answer a library probe for the given genre (a nullable, NUL-terminated C
/// string; null means "no genre filter"). Returns a newly allocated JSON C
/// string that the caller MUST release with [`ramus_siri_free`]. Returns null
/// only if the JSON contained an interior NUL (never, in practice).
///
/// # Safety
/// `genre` must be either null or a valid pointer to a NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_probe(genre: *const c_char) -> *mut c_char {
    let genre = if genre.is_null() {
        None
    } else {
        CStr::from_ptr(genre).to_str().ok().map(str::to_owned)
    };

    let json = ramus_core::siri_probe::probe_json(genre.as_deref());
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Release a string previously returned by [`ramus_siri_probe`].
///
/// # Safety
/// `ptr` must be null, or a pointer returned by [`ramus_siri_probe`] that has
/// not already been freed.
#[no_mangle]
pub unsafe extern "C" fn ramus_siri_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}
