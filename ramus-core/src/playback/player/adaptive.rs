//! The adaptive quality ladder: one override field layered on top of the
//! baseline transcode policy, stepped down when the link can't sustain the
//! stream and cleared only on a positive external event.

use std::time::{Duration, Instant};

use crate::models::{Track, TranscodeBitrate};
use crate::playback::transcode;
use crate::util::is_lossless_codec;

use super::{AudioPlayer, PlayerInner};

/// Minimum gap between two adaptive quality steps. A step costs a stream
/// swap, and the fresh stream needs a full evidence window of its own before
/// we can say whether it helped — stepping faster than that would walk
/// straight to the floor off one bad patch.
const DEGRADE_COOLDOWN: Duration = Duration::from_secs(60);

/// How much of the current track must remain for an adaptive step to be
/// applied mid-track rather than at the next boundary. Applying it costs an
/// audible gap, which is worth paying for a stream that will stutter for
/// minutes yet, but not seconds before the track changes anyway.
const DEGRADE_MIN_REMAINING: f64 = 45.0;

/// What an adaptive quality step did, for the platform layer to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandwidthDegrade {
    /// The bitrate the stream will use from now on.
    pub bitrate: TranscodeBitrate,
    /// Whether the current track was reloaded to apply it immediately, as
    /// opposed to leaving it to take effect at the next track.
    pub applied_to_current: bool,
}

/// The stream policy actually in force for `track`: whether to transcode,
/// and at what bitrate.
///
/// [`transcode::should_transcode`] is the baseline — a pure function of the
/// user's mode and the connection type. This applies the adaptive override on
/// top, and is what every resolve site must call so the live stream, the
/// prefetch worker, the byte-size estimate, the stream-record extension and
/// the debug readout can never disagree about what is being fetched.
///
/// The override only ever re-encodes something the baseline would have been
/// willing to: a lossy source has nowhere better to go, and `Never` opted out
/// of re-encoding altogether. Those cases fall through to the baseline even if
/// a degrade is somehow set.
pub(super) fn effective_stream_policy(
    track: &Track,
    inner: &PlayerInner,
) -> (bool, TranscodeBitrate) {
    let base = transcode::should_transcode(
        track.codec.as_deref(),
        inner.config.playback_mode,
        inner.is_cellular,
    );
    let Some(degraded) = inner.bandwidth_degrade else {
        return (base, inner.config.transcode_bitrate);
    };
    let eligible = base
        || (inner.config.playback_mode.adapts_to_slow_connection()
            && track.codec.as_deref().is_some_and(is_lossless_codec));
    if eligible {
        (true, degraded)
    } else {
        (base, inner.config.transcode_bitrate)
    }
}

impl AudioPlayer {
    /// The adaptive bitrate currently forced by a starving link, if any.
    pub fn bandwidth_degrade(&self) -> Option<TranscodeBitrate> {
        self.inner.lock().bandwidth_degrade
    }

    /// Drop the adaptive step and go back to the user's configured policy.
    ///
    /// Called on the events that make the old measurement meaningless: a real
    /// network path flip, a connection change or recovery, a settings change.
    /// Deliberately *not* called when starvation merely stops — see the
    /// `bandwidth_degrade` field note on why auto-restoring flaps.
    pub fn clear_bandwidth_degrade(&self) {
        let mut inner = self.inner.lock();
        if inner.bandwidth_degrade.take().is_some() {
            log::info!("bandwidth degrade cleared, back to configured policy");
        }
        inner.last_degrade_at = None;
        inner.starvation.clear();
    }

    /// Take one step down the quality ladder if the link is starving and the
    /// user's mode allows it. Returns what changed, or `None` if nothing did.
    ///
    /// Two different moves share this one path, because they are the same
    /// decision at different starting points:
    ///
    /// - **Starting** to transcode a direct-playing stream, at the user's
    ///   configured bitrate. Permitted by every mode except `Never`, whose
    ///   promise is absolute — a user who wants this behaviour picks
    ///   `WhenSlow`.
    /// - **Stepping down** an already-transcoded stream. Permitted whenever
    ///   the stream is already transcoded, since the mode that put it there
    ///   already consented to re-encoding as a bandwidth measure; this is
    ///   only choosing a smaller one.
    ///
    /// The caller owns reporting it. Applying it costs a stream swap, so it
    /// happens mid-track only when [`DEGRADE_MIN_REMAINING`] is left;
    /// otherwise the queue resweep carries it into the next track.
    pub fn consider_bandwidth_degrade(&self) -> Option<BandwidthDegrade> {
        let (next, reload) = {
            // Persistent read BEFORE the inner lock, matching every other
            // combined-lock site in this file. `parking_lot`'s RwLock is
            // task-fair — a queued writer blocks new readers — so taking
            // these in the opposite order closes a deadlock cycle against
            // any thread holding the read and waiting on `inner` (e.g.
            // `try_recover_current_track`) while a prefetch completion
            // queues `register_persistent_download`'s write.
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            let now = Instant::now();
            if !inner.starvation_verdict(now) {
                return None;
            }
            if inner
                .last_degrade_at
                .is_some_and(|t| now.duration_since(t) < DEGRADE_COOLDOWN)
            {
                return None;
            }
            let track = inner.state.queue.get(inner.state.queue_index)?.clone();
            // A local file that stutters has a problem no re-encode fixes.
            if persistent.contains_key(&track.rating_key)
                || inner.cache.get(&track.rating_key).is_some()
            {
                return None;
            }
            let (transcoding, bitrate) = effective_stream_policy(&track, &inner);
            let next = if transcoding {
                bitrate.step_down()?
            } else {
                // Nothing to start: the mode opted out, or the source is
                // already lossy and has nowhere better to go.
                if !inner.config.playback_mode.adapts_to_slow_connection()
                    || !track.codec.as_deref().is_some_and(is_lossless_codec)
                {
                    return None;
                }
                inner.config.transcode_bitrate
            };

            inner.bandwidth_degrade = Some(next);
            inner.last_degrade_at = Some(now);
            // The evidence has been acted on; the next step must be earned by
            // the new stream rather than inherited from the old one.
            inner.starvation.clear();

            let remaining = inner.duration - inner.position;
            (next, remaining >= DEGRADE_MIN_REMAINING)
        };

        log::info!(
            "connection can't sustain the stream: stepping to {} kbps",
            next.as_kbps()
        );
        // Queued entries resolved under the old policy — resweep so the step
        // lands on the next track even when the current one isn't reloaded.
        self.rewrite_stale_playlist_urls();

        let applied_to_current = reload && {
            let (resume, idx) = {
                let inner = self.inner.lock();
                (inner.position, inner.state.queue_index)
            };
            self.reload_current_track(Some(resume), Some(idx))
        };
        Some(BandwidthDegrade {
            bitrate: next,
            applied_to_current,
        })
    }
}
