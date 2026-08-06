//! Detecting a link that is up and delivering, but too slowly to keep the
//! demuxer cache fed. Measured as rebuffer episodes rather than throughput.

use std::time::{Duration, Instant};

use super::diagnostics::BUFFERING_HINT_SECS;
use super::AudioPlayer;

/// Sliding window over which starvation is judged. Long enough that a
/// single unlucky patch of a good connection can't fill it, short enough
/// that the verdict tracks the link as conditions move.
pub const STARVATION_WINDOW: Duration = Duration::from_secs(60);

/// Fraction of the observation window that must be spent silent before the
/// link counts as unable to sustain the stream. A quarter of wall-clock lost
/// to rebuffering is well past "occasional hiccup" and firmly into unlistenable.
const STARVATION_SILENT_FRACTION: f64 = 0.25;

/// Minimum number of *completed* rebuffer episodes in the window. Requiring
/// more than one is what separates starvation from a dead socket: a stream
/// that stopped and never resumed produces a single open-ended gap, whereas a
/// link that is merely too slow keeps delivering in bursts.
const STARVATION_MIN_EPISODES: usize = 2;

/// How long the current load must have been observed before any starvation
/// verdict. Opening a stream legitimately costs a few seconds of silence;
/// judging before this would call every cold start a starving link.
pub(super) const STARVATION_MIN_OBSERVATION: Duration = Duration::from_secs(30);

/// Rolling record of completed rebuffer episodes on the current load.
///
/// A "rebuffer episode" is a gap between consecutive `time-pos` events long
/// enough to mean mpv ran its demuxer cache dry ([`BUFFERING_HINT_SECS`]).
/// Episodes are recorded when they *end* — the arriving tick is the proof
/// that bytes are still being delivered — and pruned to [`STARVATION_WINDOW`].
///
/// This is the whole measurement: starvation is "the cache keeps running
/// dry", which is directly observable, rather than a throughput figure that
/// would then have to be compared against an estimated stream bitrate.
#[derive(Debug, Default)]
pub struct StarvationTracker {
    /// `(when the episode ended, how long it lasted)`, oldest first.
    episodes: Vec<(Instant, Duration)>,
}

impl StarvationTracker {
    pub(super) fn record(&mut self, now: Instant, gap: Duration) {
        self.episodes
            .retain(|(t, _)| now.saturating_duration_since(*t) <= STARVATION_WINDOW);
        self.episodes.push((now, gap));
    }

    pub(super) fn clear(&mut self) {
        self.episodes.clear();
    }

    pub(super) fn episodes(&self) -> &[(Instant, Duration)] {
        &self.episodes
    }

    /// Episodes still inside the window. Pruning only happens on `record`,
    /// so a read-time filter is needed for an honest count.
    pub(super) fn recent_count(&self, now: Instant) -> usize {
        self.episodes
            .iter()
            .filter(|(t, _)| now.saturating_duration_since(*t) <= STARVATION_WINDOW)
            .count()
    }
}

/// Whether the link is failing to sustain the current stream.
///
/// Free function beside [`derive_phase`] for the same reason: the verdict is
/// pure, so it can be unit-tested without an mpv handle or a player lock.
///
/// `in_progress_gap` is the currently-open silence (`now` minus the last
/// tick), counted alongside the completed episodes so a verdict isn't delayed
/// until the stream happens to resume. `observed_for` is how long the current
/// load has been running, which both gates the cold start and caps the
/// denominator — 15 s of silence means something different in the first
/// 40 s of a track than across a full minute.
pub fn is_starving(
    episodes: &[(Instant, Duration)],
    in_progress_gap: Duration,
    observed_for: Duration,
    now: Instant,
) -> bool {
    if observed_for < STARVATION_MIN_OBSERVATION {
        return false;
    }
    let recent: Vec<Duration> = episodes
        .iter()
        .filter(|(t, _)| now.saturating_duration_since(*t) <= STARVATION_WINDOW)
        .map(|(_, d)| *d)
        .collect();
    if recent.len() < STARVATION_MIN_EPISODES {
        return false;
    }
    let mut silent: Duration = recent.iter().sum();
    if in_progress_gap >= Duration::from_secs(BUFFERING_HINT_SECS) {
        silent += in_progress_gap;
    }
    let window = STARVATION_WINDOW.min(observed_for);
    silent.as_secs_f64() >= window.as_secs_f64() * STARVATION_SILENT_FRACTION
}

impl AudioPlayer {
    /// Whether the link is delivering but too slowly to sustain the current
    /// stream — repeated rebuffering rather than a clean stop. See
    /// [`is_starving`].
    ///
    /// A reload cannot fix this (it re-resolves to the same stream and throws
    /// away whatever was buffered), so recovery paths consult this before
    /// acting, and the prefetch worker consults it before taking bandwidth
    /// the live stream is already short of.
    pub fn is_starving(&self) -> bool {
        self.inner.lock().starvation_verdict(Instant::now())
    }
}
