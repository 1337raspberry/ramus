//! C entry point for home-screen widget buttons. The buttons are App
//! Intents that iOS runs in the app's process, launching it in the
//! background when it isn't running; the intent calls
//! `ramus_widget_command`, which waits for `setup()` to finish before
//! acting, so a button pressed on a cold launch plays the restored queue
//! instead of finding no player.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use ramus_core::widget::WidgetCommand;
use tauri::AppHandle;

pub const WIDGET_OK: i32 = 0;
pub const WIDGET_NOT_READY: i32 = 1;
pub const WIDGET_BAD_INPUT: i32 = 2;
pub const WIDGET_FAILED: i32 = 3;
pub const WIDGET_DROPPED: i32 = 4;

/// Set once at the end of `setup()`; commands wait on it.
pub(crate) struct ReadyCell<T> {
    value: OnceLock<T>,
    signal: (Mutex<()>, Condvar),
}

impl<T: Clone> ReadyCell<T> {
    pub(crate) const fn new() -> Self {
        Self {
            value: OnceLock::new(),
            signal: (Mutex::new(()), Condvar::new()),
        }
    }

    pub(crate) fn set(&self, value: T) {
        let _ = self.value.set(value);
        let _guard = self.signal.0.lock().unwrap_or_else(|e| e.into_inner());
        self.signal.1.notify_all();
    }

    /// The value, waiting up to `timeout` for it to be set.
    pub(crate) fn wait(&self, timeout: Duration) -> Option<T> {
        if let Some(v) = self.value.get() {
            return Some(v.clone());
        }
        let guard = self.signal.0.lock().unwrap_or_else(|e| e.into_inner());
        let (_guard, _) = self
            .signal
            .1
            .wait_timeout_while(guard, timeout, |_| self.value.get().is_none())
            .unwrap_or_else(|e| e.into_inner());
        self.value.get().cloned()
    }
}

/// The app handle, set once `setup()` has finished.
static APP: ReadyCell<AppHandle> = ReadyCell::new();

/// Called at the end of `setup()`, once the session and queue have been
/// restored and `AppState` is managed.
pub fn mark_ready(app: AppHandle) {
    APP.set(app);
}

pub(crate) fn parse_command(json: &str) -> Result<WidgetCommand, i32> {
    serde_json::from_str(json).map_err(|e| {
        log::warn!("widget command rejected: {e}");
        WIDGET_BAD_INPUT
    })
}

/// Run one widget command, given as JSON (see `WidgetCommand`). Blocks the
/// calling thread until the command has been carried out, waiting up to
/// `timeout_ms` for the app to be ready first. Returns one of the
/// `WIDGET_*` codes. Called from Swift on a background dispatch queue,
/// never from the main thread.
///
/// # Safety
///
/// `json` must be a valid, NUL-terminated C string that stays alive for
/// the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn ramus_widget_command(
    json: *const std::ffi::c_char,
    timeout_ms: u32,
) -> i32 {
    if json.is_null() {
        return WIDGET_BAD_INPUT;
    }
    let Ok(json) = std::ffi::CStr::from_ptr(json).to_str() else {
        return WIDGET_BAD_INPUT;
    };
    let cmd = match parse_command(json) {
        Ok(cmd) => cmd,
        Err(code) => return code,
    };
    let Some(app) = APP.wait(Duration::from_millis(timeout_ms as u64)) else {
        log::warn!("widget command arrived before the app was ready");
        return WIDGET_NOT_READY;
    };
    guarded(|| tauri::async_runtime::block_on(super::actions::run_command(&app, cmd)))
}

/// Run `f`, turning a panic into `WIDGET_FAILED`: unwinding out of an
/// `extern "C"` function aborts the whole process.
fn guarded(f: impl FnOnce() -> i32) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("widget command panicked");
        WIDGET_FAILED
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_command_reports_failure() {
        assert_eq!(guarded(|| panic!("boom")), WIDGET_FAILED);
        assert_eq!(guarded(|| WIDGET_OK), WIDGET_OK);
    }

    #[test]
    fn not_ready_times_out() {
        let cell: ReadyCell<u32> = ReadyCell::new();
        assert_eq!(cell.wait(Duration::from_millis(10)), None);
    }

    #[test]
    fn ready_wakes_a_waiter() {
        static CELL: ReadyCell<u32> = ReadyCell::new();
        let waiter = std::thread::spawn(|| CELL.wait(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(20));
        CELL.set(7);
        assert_eq!(waiter.join().unwrap(), Some(7));
        assert_eq!(CELL.wait(Duration::ZERO), Some(7));
    }

    #[test]
    fn bad_json_is_bad_input() {
        assert_eq!(parse_command("{"), Err(WIDGET_BAD_INPUT));
        assert_eq!(
            parse_command(r#"{"action":"dance"}"#),
            Err(WIDGET_BAD_INPUT)
        );
        assert_eq!(
            parse_command(r#"{"action":"resume"}"#),
            Ok(WidgetCommand::Resume)
        );
    }
}
