//! What the player is actually doing, as opposed to what its optimistic
//! status flag claims: the derived phase, the debug snapshot, the
//! lock-screen position snapshot, and the demuxer drain checks.

use std::time::{Duration, Instant};

use crate::models::{PlaybackMode, PlaybackStatus};

use super::adaptive::effective_stream_policy;
use super::AudioPlayer;

/// Time after a load with no `time-pos` updates before `derive_phase` flips
/// from `Buffering` to `Stalled`. The frontend uses this to colour the row,
/// the watchdog uses it to trigger a connection re-evaluation.
pub const STALL_THRESHOLD_SECS: u64 = 12;

/// Mid-track: seconds without a `time-pos` update before `derive_phase`
/// reports `Buffering` instead of `Playing`. Position events flow several
/// times a second while audio runs, so a few silent seconds means mpv is
/// genuinely rebuffering — but not yet long enough to call it a `Stalled`
/// (which is what triggers the watchdog's connection re-evaluation).
pub const BUFFERING_HINT_SECS: u64 = 3;

/// Derived playback phase shown in the debug panel. Captures what mpv is
/// actually doing rather than the optimistic `PlaybackStatus` flag the rest
/// of the app uses.
///
/// `Status::Playing` flips the moment `load_queue` runs, before mpv has even
/// opened the URL — `Phase` distinguishes that from the post-`file-loaded`
/// state where time-pos is actively advancing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Stopped,
    Paused,
    /// `load_queue` ran, mpv hasn't fired `file-loaded` yet.
    Opening,
    /// mpv loaded the file but we haven't seen a position update yet.
    Buffering,
    /// position updates arriving normally.
    Playing,
    /// status is Playing but no progress for `STALL_THRESHOLD_SECS`.
    Stalled,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugInfo {
    pub source: String,
    pub resolved_url: Option<String>,
    pub server_url: Option<String>,
    /// Active connection is non-local. Diagnostic only — feeds no policy.
    pub is_remote: bool,
    /// Whether the platform network monitor currently reports a cellular
    /// path. Always `false` on desktop.
    pub is_cellular: bool,
    pub playback_mode: PlaybackMode,
    pub queue_len: usize,
    pub queue_index: usize,
    pub lookahead_depth: u8,
    pub cached_in_lookahead: u32,
    /// Subset of `cached_in_lookahead` whose on-disk file is a transcoded
    /// (`.ogg`) prefetch — populated by `build_transcode_download_url` on
    /// the prefetch path. Caveat: a track whose original codec is also Ogg
    /// will mislabel here, but that's vanishingly rare in Plex libraries.
    pub cached_in_lookahead_transcoded: u32,
    /// Subset of `cached_in_lookahead` whose file is a direct-play copy
    /// (anything other than `.ogg`).
    pub cached_in_lookahead_direct: u32,
    pub total_in_lookahead: u32,
    pub codec: Option<String>,
    pub bitrate: Option<i32>,
    pub file_size_bytes: Option<i64>,
    /// Derived phase — preferred over `status` when reasoning about whether
    /// audio is actually flowing.
    pub phase: Phase,
    /// The link is up and delivering, but too slowly to sustain this stream
    /// (repeated rebuffering). Distinct from `phase: Stalled`, which a dead
    /// socket also produces.
    pub starving: bool,
    /// Bitrate the adaptive layer has forced this session, in kbps, or
    /// `None` when playing under the user's configured policy.
    pub degraded_to_kbps: Option<u16>,
    /// mpv's raw `demuxer-cache-time` — seconds of media buffered ahead.
    ///
    /// This is the signal the starvation discriminator reads, surfaced so it
    /// can be checked against reality: during a stall it should keep climbing
    /// while bytes are still arriving (however slowly), and sit frozen once
    /// the socket is dead. `None` where the bridge can't report it.
    pub demuxer_cache_time: Option<f64>,
    /// Completed rebuffer episodes inside the starvation window — the
    /// evidence accumulating toward (or ageing out of) the `starving`
    /// verdict, visible before it flips.
    pub starvation_episodes: usize,
    /// mpv holds the whole track. A frozen `demuxer_cache_time` means
    /// something very different here (the source finished, the healthy end
    /// state) than it does mid-stream (the source stopped delivering).
    pub source_drained: bool,
    /// Seconds since the last `time-pos` event, or `None` if the current
    /// load hasn't produced one yet.
    pub seconds_since_position_update: Option<u64>,
    /// Seconds since the current track started loading.
    pub seconds_since_load: Option<u64>,
    /// Last `MPV_EVENT_END_FILE` reason seen with `Error`. Cleared on the
    /// next successful `file-loaded`. Already URL-redacted.
    pub last_load_error: Option<String>,
}

/// Derive a `Phase` from the optimistic `PlaybackStatus` plus the load-time
/// and position-update timestamps. Encapsulated as a free function so the
/// stall watchdog can call it without holding the player lock for any
/// longer than it takes to copy three small values.
pub fn derive_phase(
    status: PlaybackStatus,
    last_position_update: Option<Instant>,
    load_started_at: Option<Instant>,
    now: Instant,
) -> Phase {
    match status {
        PlaybackStatus::Stopped => Phase::Stopped,
        PlaybackStatus::Paused => Phase::Paused,
        PlaybackStatus::Playing => match (last_position_update, load_started_at) {
            (Some(t), _) => {
                let secs = now.saturating_duration_since(t).as_secs();
                if secs >= STALL_THRESHOLD_SECS {
                    Phase::Stalled
                } else if secs >= BUFFERING_HINT_SECS {
                    // Mid-track position ticks have dried up but not long
                    // enough to call it a stall: mpv is rebuffering (cache
                    // ran dry on a slow link). Without this rung the phase
                    // claims Playing for the full pre-stall window while no
                    // audio flows.
                    Phase::Buffering
                } else {
                    Phase::Playing
                }
            }
            (None, Some(load)) => {
                if now.saturating_duration_since(load).as_secs() >= STALL_THRESHOLD_SECS {
                    Phase::Stalled
                } else {
                    Phase::Buffering
                }
            }
            // Unreachable in practice: every path that sets status=Playing
            // also stamps `load_started_at`. Kept as a total-match fallback.
            (None, None) => Phase::Opening,
        },
    }
}

/// Slack on the fully-buffered comparison, covering float jitter and mpv's
/// approximate final timestamp.
const DRAIN_SLACK: f64 = 0.25;

/// Whether mpv has pulled the whole track into its demuxer cache.
///
/// `demuxer-cache-time` is the **timestamp of the last buffered packet** — an
/// absolute point on the track's timeline, not an amount buffered ahead of the
/// play head. So the comparison is against the full track duration and must
/// not subtract the current position: doing that declares a source drained as
/// soon as the cache end passes `duration - position`, which mid-track is far
/// short of the real end (at the half-way point it fires with only a quarter
/// of the track buffered).
///
/// `duration` must be the Plex-DB `Track.duration`, not mpv's reported value —
/// mpv's estimate for a chunked Ogg transcode grows in lockstep with
/// `demuxer-cache-time`, so comparing the two always returns true.
///
/// Free function so [`AudioPlayer::current_source_fully_drained`] and the
/// debug snapshot share one definition; the snapshot can't call the method,
/// because it already holds the (non-reentrant) player lock.
pub(super) fn source_fully_buffered(cache_time: Option<f64>, duration: f64) -> bool {
    let Some(cache_time) = cache_time else {
        return false;
    };
    if duration <= 0.0 {
        return false;
    }
    cache_time >= duration - DRAIN_SLACK
}

/// Snapshot of playback progress for the lock-screen now-playing keeper,
/// returned by [`AudioPlayer::media_position_snapshot`].
#[derive(Debug, Clone, Copy)]
pub struct MediaPositionSnapshot {
    /// The player believes it should be playing (status == Playing).
    pub is_playing: bool,
    /// Playing, at least one position tick has arrived for the current load,
    /// but none within the requested threshold — mid-track audio has stalled
    /// (a network hiccup or the reload gap on a failover resume). The OS
    /// scrubber must be frozen at `position` while this holds, else it keeps
    /// extrapolating forward as if still playing.
    pub progress_stalled: bool,
    /// Last known true playback position (already `position_base`-remapped).
    pub position: f64,
}

impl AudioPlayer {
    pub fn debug_snapshot(&self) -> DebugInfo {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let track = inner.state.queue.get(inner.state.queue_index);

        let (source, resolved_url) = match track {
            Some(t) => {
                if persistent.contains_key(&t.rating_key) {
                    ("downloaded".into(), persistent.get(&t.rating_key)
                        .map(|p| format!("file://{}", p.display())))
                } else if let Some(path) = inner.cache.get(&t.rating_key) {
                    ("cached".into(), Some(format!("file://{}", path.display())))
                } else if effective_stream_policy(t, &inner).0 {
                    ("transcode".into(), inner.server_url.as_ref().map(|u| {
                        format!("{}/audio/:/transcode/…", u.as_str().trim_end_matches('/'))
                    }))
                } else {
                    ("streaming".into(), t.part_key.as_ref().and_then(|pk| {
                        inner.server_url.as_ref().map(|u| {
                            format!("{}{}", u.as_str().trim_end_matches('/'), pk)
                        })
                    }))
                }
            }
            None => ("none".into(), None),
        };

        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let mut cached_in_lookahead = 0u32;
        let mut cached_in_lookahead_transcoded = 0u32;
        let mut cached_in_lookahead_direct = 0u32;
        let mut total_in_lookahead = 0u32;
        for offset in 1..=depth {
            let Some(t) = inner.state.queue.get(pos + offset) else {
                break;
            };
            total_in_lookahead += 1;
            let path = persistent
                .get(&t.rating_key)
                .cloned()
                .or_else(|| inner.cache.get(&t.rating_key).map(|p| p.to_path_buf()));
            if let Some(p) = path {
                cached_in_lookahead += 1;
                let is_transcoded = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("ogg"))
                    .unwrap_or(false);
                if is_transcoded {
                    cached_in_lookahead_transcoded += 1;
                } else {
                    cached_in_lookahead_direct += 1;
                }
            }
        }

        let now = Instant::now();
        let cache_time = self.mpv.demuxer_cache_time();
        let phase = derive_phase(
            inner.state.status,
            inner.last_position_update,
            inner.load_started_at,
            now,
        );

        DebugInfo {
            source,
            resolved_url,
            server_url: inner.server_url.as_ref().map(|u| u.to_string()),
            is_remote: inner.is_remote,
            is_cellular: inner.is_cellular,
            playback_mode: inner.config.playback_mode,
            queue_len: inner.state.queue.len(),
            queue_index: inner.state.queue_index,
            lookahead_depth: inner.config.lookahead_depth,
            cached_in_lookahead,
            cached_in_lookahead_transcoded,
            cached_in_lookahead_direct,
            total_in_lookahead,
            codec: track.and_then(|t| t.codec.clone()),
            bitrate: track.and_then(|t| t.bitrate),
            file_size_bytes: track.and_then(|t| t.file_size_bytes),
            phase,
            starving: inner.starvation_verdict(now),
            degraded_to_kbps: inner.bandwidth_degrade.map(|b| b.as_kbps()),
            // Read under the lock, as `current_source_fully_drained` does.
            // Only reached from `get_debug_info`, polled at 1 Hz while the
            // panel is open, so the extra property read costs nothing.
            demuxer_cache_time: cache_time,
            starvation_episodes: inner.starvation.recent_count(now),
            // Can't call `current_source_fully_drained()` here — it takes the
            // player lock this snapshot already holds, and `parking_lot`'s
            // Mutex is not reentrant. Shared free function instead.
            source_drained: source_fully_buffered(
                cache_time,
                track.map(|t| t.duration).unwrap_or(0.0),
            ),
            seconds_since_position_update: inner
                .last_position_update
                .map(|t| now.saturating_duration_since(t).as_secs()),
            seconds_since_load: inner
                .load_started_at
                .map(|t| now.saturating_duration_since(t).as_secs()),
            last_load_error: inner.last_load_error.clone(),
        }
    }

    /// Snapshot the current playback progress for the lock-screen now-playing
    /// keeper. `stall_threshold` is how long without a `time-pos` tick (while
    /// Playing) counts as a mid-track stall; before the first tick of a load
    /// the threshold is anchored on the load start instead, so a failed open
    /// still freezes the OS scrubber. Distinct from [`is_stalled`], which
    /// uses the longer connection-recovery threshold.
    pub fn media_position_snapshot(&self, stall_threshold: Duration) -> MediaPositionSnapshot {
        let inner = self.inner.lock();
        let is_playing = inner.state.status == PlaybackStatus::Playing;
        let progress_stalled = is_playing
            && match inner.last_position_update {
                Some(t) => t.elapsed() >= stall_threshold,
                // No tick has yet arrived for this load — anchor the freeze
                // on the load start instead. A failed open (or a cascade of
                // them) never produces a first tick, and without this rung
                // the OS scrubber sails forward "playing @ 0:00" through the
                // whole failure because the keeper had nothing to compare
                // against. Genuine quick opens tick well inside the
                // threshold, so normal startup never trips it.
                None => inner
                    .load_started_at
                    .is_some_and(|t| t.elapsed() >= stall_threshold),
            };
        MediaPositionSnapshot {
            is_playing,
            progress_stalled,
            position: inner.position,
        }
    }

    /// Whether mpv has had no `time-pos` activity for `STALL_THRESHOLD_SECS`
    /// while we believe it should be playing. Used by the stall watchdog
    /// to fire a connection re-evaluation (the only thing that might
    /// recover a stuck transcode session against an unreachable host).
    pub fn is_stalled(&self) -> bool {
        let inner = self.inner.lock();
        derive_phase(
            inner.state.status,
            inner.last_position_update,
            inner.load_started_at,
            Instant::now(),
        ) == Phase::Stalled
    }

    /// Whether the currently-playing track's source bytes have been fully
    /// pulled by mpv (i.e. the source HTTP body has EOFed and mpv is
    /// playing from its in-memory buffer the rest of the way through).
    ///
    /// Compares `demuxer-cache-time` against the Plex-DB `Track.duration`
    /// (immutable, set at sync time), NOT mpv's reported `inner.duration`.
    /// mpv's estimate grows in step with `demuxer-cache-time` while
    /// buffering a chunked Ogg stream, so checking against it always
    /// returns true. The DB duration breaks that coupling. 0.25s slack
    /// covers float jitter.
    ///
    /// Note: this is a "demuxer is at-or-near end" check, not a "file
    /// on disk is structurally complete" check. The recorder's
    /// libavformat may still hold tail packets in its internal page
    /// buffer even after the demuxer reports drained — callers needing
    /// a structurally valid file should clamp reads at the last
    /// complete Ogg page boundary (see `bounded_ogg_source` in the
    /// spectrum analyser) rather than relying on this predicate alone.
    ///
    /// Returns `false` if the queue is empty, the track has no DB
    /// duration, or the bridge doesn't expose `demuxer-cache-time`.
    pub fn current_source_fully_drained(&self) -> bool {
        let inner = self.inner.lock();
        let Some(track) = inner.state.queue.get(inner.state.queue_index) else {
            return false;
        };
        source_fully_buffered(self.mpv.demuxer_cache_time(), track.duration)
    }

    /// Raw `demuxer-cache-time` from the underlying mpv bridge, exposed
    /// for callers (the prefetch worker) that need to track changes
    /// across polls — e.g. confirming the demuxer has actually stopped
    /// pulling, not just "almost there".
    ///
    /// `None` when the bridge can't report the property (mobile bridges
    /// that haven't grown the call yet) or mpv has nothing buffered.
    pub fn demuxer_cache_time(&self) -> Option<f64> {
        self.mpv.demuxer_cache_time()
    }

    /// Approximate expected on-disk size for the currently-playing
    /// track's source body, in bytes. Drain detection compares this
    /// against the actual stream-record file size to decide whether
    /// Plex has finished sending the body.
    ///
    /// For transcoded tracks: `duration × transcode_bitrate / 8`. The
    /// Opus encoder is VBR so this is approximate (callers should use a
    /// 95% threshold). For direct-play: prefers the exact
    /// `Track.file_size_bytes` populated at sync time, falling back to
    /// `duration × Track.bitrate / 8` if the column wasn't populated.
    ///
    /// Returns `None` when the queue is empty, the track has no
    /// duration, or no usable bitrate / size hint is available.
    pub fn expected_source_bytes_for_current(&self) -> Option<u64> {
        let inner = self.inner.lock();
        let track = inner.state.queue.get(inner.state.queue_index)?;
        if track.duration <= 0.0 {
            return None;
        }
        let (needs_transcode, bitrate) = effective_stream_policy(track, &inner);
        if needs_transcode {
            let kbps = bitrate.as_kbps() as f64;
            Some((track.duration * kbps * 1000.0 / 8.0) as u64)
        } else if let Some(sz) = track.file_size_bytes.filter(|s| *s > 0) {
            Some(sz as u64)
        } else {
            let kbps = track.bitrate.filter(|b| *b > 0)? as f64;
            Some((track.duration * kbps * 1000.0 / 8.0) as u64)
        }
    }
}
