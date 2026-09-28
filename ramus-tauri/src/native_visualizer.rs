//! Hand-off of the live spectrum to the native full-screen visualiser.
//!
//! On iOS the full-screen visualiser can be drawn natively over the web
//! view. While it is showing, the decoded, level-mapped spectrum frames and
//! audible-clock ticks go to the plugin instead of to the web page: the
//! spectrum callbacks queue them here, and one forwarding thread hands
//! them to Swift. The callbacks run inside a plugin event (the tap's log
//! lines arrive through a Swift `trigger`), so they never call the plugin
//! themselves; the queue drops its oldest item rather than ever blocking
//! them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ramus_core::util::DropOldestQueue;

use crate::events::{PlaybackAudiblePayload, SpectrumFramesPayload};

/// Pushes held for the forwarding thread; beyond this the oldest is dropped.
const QUEUE_CAPACITY: usize = 8;

/// One item for the native visualiser.
pub enum NativeVisualizerPush {
    Frames(SpectrumFramesPayload),
    Audible(PlaybackAudiblePayload),
}

/// Whether the native visualiser is showing, and the queue of pushes
/// waiting for the forwarding thread.
pub struct NativeVisualizerLink {
    active: AtomicBool,
    queue: DropOldestQueue<NativeVisualizerPush>,
    /// Cleared on every activation; set by the first push failure after it,
    /// so that one logs at `warn` and the rest log at `debug` until the
    /// next activation. Only read on iOS, where the forwarder thread runs.
    #[cfg(target_os = "ios")]
    warned_since_activation: AtomicBool,
}

impl NativeVisualizerLink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            active: AtomicBool::new(false),
            queue: DropOldestQueue::new(QUEUE_CAPACITY),
            #[cfg(target_os = "ios")]
            warned_since_activation: AtomicBool::new(false),
        })
    }

    /// True while the native visualiser is showing: the spectrum callbacks
    /// forward instead of emitting to the web page.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Set once the native view is up; cleared before it is taken down,
    /// which also drops anything still queued for it.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
        if !active {
            self.queue.clear();
        }
        #[cfg(target_os = "ios")]
        if active {
            self.warned_since_activation.store(false, Ordering::Release);
        }
    }

    /// Queue `push` for the forwarding thread. Never blocks.
    pub fn forward(&self, push: NativeVisualizerPush) {
        self.queue.push(push);
    }

    /// Whether a push failure should log at `warn` rather than `debug`:
    /// true for the first failure since the last activation. Lock-free —
    /// two failures racing on the flag can both see `false` and both log at
    /// `warn`, which is harmless for a diagnostic message.
    #[cfg(target_os = "ios")]
    fn should_warn(&self) -> bool {
        self.warned_since_activation
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    #[cfg(test)]
    fn try_next(&self, timeout: std::time::Duration) -> Option<NativeVisualizerPush> {
        self.queue.pop_timeout(timeout)
    }
}

/// Start the thread that hands queued pushes to the plugin. Runs for the
/// life of the app. This side can only check `is_active` before dispatching
/// — it can't make that check-then-send atomic with a hide arriving in
/// between — so the actual drop-after-hide guarantee is Swift's: hide
/// clears the feed on the command queue first, and a push that reaches the
/// plugin afterwards finds no feed to write into.
#[cfg(target_os = "ios")]
pub fn spawn_forwarder(app: tauri::AppHandle, link: Arc<NativeVisualizerLink>) {
    use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
    let spawned = std::thread::Builder::new()
        .name("native-visualizer".into())
        .spawn(move || loop {
            let push = link.queue.pop();
            if !link.is_active() {
                continue;
            }
            let bridge = app.ramus_ios_bridge();
            let result = match &push {
                NativeVisualizerPush::Frames(p) => bridge.push_spectrum_frames(p),
                NativeVisualizerPush::Audible(p) => bridge.push_audible(p),
            };
            if let Err(e) = result {
                // Only the first failure per activation is loud: a wire
                // mismatch would otherwise show on a device as a silently
                // flat ridge with nothing in the log to explain it.
                if link.should_warn() {
                    log::warn!("native visualiser push failed: {e}");
                } else {
                    log::debug!("native visualiser push failed: {e}");
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("native visualiser forwarder failed to start: {e}");
    }
}

/// Relay the native visualiser's tap-to-close to the page as
/// `visualizer-dismiss`.
#[cfg(target_os = "ios")]
pub fn register_dismiss_listener(
    app: &tauri::AppHandle,
) -> tauri_plugin_ramus_ios_bridge::Result<()> {
    use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
    let handle = app.clone();
    let channel = tauri::ipc::Channel::new(move |_body| {
        crate::events::emit_visualizer_dismiss(&handle);
        Ok(())
    });
    app.ramus_ios_bridge()
        .register_listener("nativeVisualizerDismiss", channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tick(position: f64) -> NativeVisualizerPush {
        NativeVisualizerPush::Audible(PlaybackAudiblePayload { epoch: 1, position })
    }

    #[test]
    fn a_new_link_is_inactive() {
        assert!(!NativeVisualizerLink::new().is_active());
    }

    #[test]
    fn forwarded_pushes_are_queued_in_order() {
        let link = NativeVisualizerLink::new();
        link.set_active(true);
        link.forward(tick(1.0));
        link.forward(tick(2.0));
        let positions: Vec<f64> = (0..2)
            .map(|_| match link.try_next(Duration::from_millis(10)) {
                Some(NativeVisualizerPush::Audible(p)) => p.position,
                _ => panic!("expected an audible tick"),
            })
            .collect();
        assert_eq!(positions, vec![1.0, 2.0]);
    }

    #[test]
    fn deactivating_drops_queued_pushes() {
        let link = NativeVisualizerLink::new();
        link.set_active(true);
        link.forward(tick(1.0));
        link.set_active(false);
        assert!(link.try_next(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn a_slow_consumer_keeps_the_newest_pushes() {
        let link = NativeVisualizerLink::new();
        link.set_active(true);
        for i in 0..(QUEUE_CAPACITY + 3) {
            link.forward(tick(i as f64));
        }
        match link.try_next(Duration::from_millis(10)) {
            Some(NativeVisualizerPush::Audible(p)) => assert_eq!(p.position, 3.0),
            _ => panic!("expected an audible tick"),
        }
    }
}
