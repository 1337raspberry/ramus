//! Download worker: serial track downloads with resumable requests.
//!
//! Handles two kinds of work, in priority order:
//!
//! 1. **User-requested downloads** (the Downloads feature) — files go to
//!    `config_dir()/downloads/<ratingKey>.<ext>`, persist forever, and
//!    register into `AudioPlayer::persistent_cache` so `resolve_url`
//!    always plays them locally.
//! 2. **Prefetch** of upcoming queue tracks — files go to
//!    `config_dir()/audio_cache/<ratingKey>_<len>.<ext>`, subject to LRU
//!    eviction. Keeps gapless playback gapless when network is slow.
//!
//! A single long-lived tokio task processes both queues serially. Plex
//! cuts off concurrent downloads from the same client on remote
//! connections (see memory note `project_plex_remote_downloads.md`), so
//! we never have more than one HTTP request in flight.
//!
//! Also generates per-track FFT spectrograms (`.spec` files) after every
//! successful download for the focus-mode visualiser.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use ramus_core::cache::downloads::DownloadRow;
use ramus_core::playback::player::{is_allowed_extension, sanitize_filename, AudioPlayer};
use ramus_core::playback::spectrum::{read_spec_file, spec_file_path};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::{
    emit_download_progress, emit_downloads_changed, emit_spectrum_ready, DownloadProgressPayload,
};
use crate::ios_backup;
use crate::spectrum_analyzer;

// --- Tunables ---

/// Per-track download time budget. Retries as many times as needed within
/// this window. Fast LAN finishes in 1–2 retries; throttled connections
/// (e.g. Windows Defender network inspection) need dozens of small-chunk
/// resumes. 90s covers a 40MB FLAC at ~500KB/s effective throughput.
const DOWNLOAD_TIME_BUDGET: Duration = Duration::from_secs(90);

/// Initial backoff between resume attempts. Doubles on consecutive retries
/// that make no progress, resets when real progress is made.
const INITIAL_BACKOFF: Duration = Duration::from_millis(200);

/// Maximum backoff between resume attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// A retry must gain at least this many bytes to count as progress for
/// backoff-reset purposes.
const MIN_PROGRESS_BYTES: u64 = 4096;

/// Hard cap on a single download. Generous enough to cover any plausible
/// lossless album track (a 24-bit/192k FLAC tops out around ~150 MB), but
/// finite — protects against a server that lies about (or omits)
/// Content-Length and streams unbounded bytes, which would otherwise fill
/// the device's disk before the time budget elapses.
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Minimum delay before starting prefetch after a natural advance, so
/// mpv has a chance to issue its initial request and report some
/// duration / position state. The actual wait extends past this if the
/// live transcode HTTP body is still draining — see
/// `wait_for_source_drain`.
const NATURAL_GAP: Duration = Duration::from_secs(1);

/// Same idea after a user skip, with a touch more so rapid skips don't
/// fire pointless downloads.
const SKIP_GAP: Duration = Duration::from_secs(2);

/// Hard ceiling on how long we'll wait for the live transcode body to
/// drain before proceeding. Plex transcodes that arrive faster than
/// realtime drain in 5–15s; transcodes that arrive at near-realtime
/// pace would never drain within ceiling because the source is
/// actively delivering the whole way through, so a long ceiling just
/// holds up the in-cycle ingest (and the visualiser the user is
/// waiting on) for nothing. 30s is enough for fast bursts to land and
/// short enough that slow-Plex cases proceed to a partial-file
/// in-cycle ingest within a tolerable wait. After ceiling, the
/// bounded Ogg reader handles whatever's on disk; the track-end
/// re-ingest catches any later growth.
const LIVE_DRAIN_CEILING: Duration = Duration::from_secs(30);

/// Poll interval for the live-drain wait. Cheap (a single mpv property
/// read per tick), so a tight cadence makes the post-drain prefetch
/// fire promptly.
const LIVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Emit progress events at most this often per in-flight download. Bytes
/// ticks at chunk rate (>50Hz on LAN); throttling keeps the event bus quiet.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(250);

// --- Public types ---

/// Build a `DownloadProgressPayload` by cloning the identity fields from a
/// `UserDownloadJob`. Keeps the four progress-emit sites inside
/// `run_user_download` short and makes changes to the payload shape a
/// single edit.
fn progress_payload(
    job: &UserDownloadJob,
    phase: &'static str,
    bytes_written: u64,
    total_bytes: Option<u64>,
    error: Option<String>,
) -> DownloadProgressPayload {
    DownloadProgressPayload {
        rating_key: job.rating_key.clone(),
        album_rating_key: job.album_rating_key.clone(),
        title: job.title.clone(),
        artist_name: job.artist_name.clone(),
        album_title: job.album_title.clone(),
        thumb: job.thumb.clone(),
        phase,
        bytes_written,
        total_bytes,
        error,
    }
}

/// Queued user-initiated download. Built by the download commands after
/// looking up track metadata and a live server URL.
#[derive(Debug, Clone)]
pub struct UserDownloadJob {
    pub rating_key: String,
    pub album_rating_key: String,
    pub title: String,
    pub artist_name: String,
    pub album_title: String,
    pub thumb: Option<String>,
    pub codec: String,
    pub url: String,
    /// Expected bytes, from `tracks.fileSizeBytes`. Used to show accurate
    /// progress bars while the HTTP response is still reading headers.
    pub expected_size_bytes: Option<u64>,
}

/// Read-only snapshot for the Downloads panel.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadManagerSnapshot {
    pub in_progress: Option<InProgressDownload>,
    /// Queued rating keys in FIFO order.
    pub queued: Vec<String>,
    /// Total items in the user queue — cheaper than `queued.len()` for
    /// callers that only care about the count.
    pub queue_len: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InProgressDownload {
    pub rating_key: String,
    pub album_rating_key: String,
    pub title: String,
    pub artist_name: String,
    pub album_title: String,
    pub thumb: Option<String>,
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
}

// --- Commands ---

#[derive(Debug, Clone)]
enum PrefetchCmd {
    /// mpv's playlist-pos advanced naturally (gapless auto-advance or
    /// play_tracks kicking off a fresh queue).
    NaturalAdvance { generation: u64 },
    /// User-initiated skip (next/prev/jump). Aborts any in-flight download
    /// and restarts with a skip gap.
    Skipped { generation: u64 },
    /// Album switch / stop. Aborts in-flight and waits for the next command.
    #[allow(dead_code)]
    Cancel { generation: u64 },
    /// A user-initiated download was queued. Wakes the worker so it spawns
    /// a cycle if idle.
    UserDownloadQueued { generation: u64 },
    /// Cancel a specific (or all) user download(s). The handle has already
    /// removed the matching entries from shared state; the worker just
    /// aborts the current cycle so in-flight work (if any) stops, then
    /// starts a fresh cycle that picks up whatever remains.
    CancelUserDownload { generation: u64 },
}

// --- Shared state ---
//
// Worker + command handlers both read/write this behind a `parking_lot::Mutex`.
// Holds: the pending user queue and the currently-in-flight item.

struct Shared {
    user_queue: VecDeque<UserDownloadJob>,
    in_progress: Option<InProgressDownload>,
}

impl Shared {
    fn new() -> Self {
        Self {
            user_queue: VecDeque::new(),
            in_progress: None,
        }
    }

    fn snapshot(&self) -> DownloadManagerSnapshot {
        // Cap the returned queue list — a bulk "download all starred" run
        // can have 1000+ items and the UI only needs to show a small
        // preview. `queue_len` keeps the total count available.
        const PREVIEW_LIMIT: usize = 64;
        DownloadManagerSnapshot {
            in_progress: self.in_progress.clone(),
            queued: self
                .user_queue
                .iter()
                .take(PREVIEW_LIMIT)
                .map(|j| j.rating_key.clone())
                .collect(),
            queue_len: self.user_queue.len(),
        }
    }
}

// --- Handle ---

/// Control surface for the background prefetch / download worker. Cloneable
/// so it can live in `AppState` and be called from any command handler or
/// event callback.
#[derive(Clone)]
pub struct PrefetchHandle {
    tx: mpsc::UnboundedSender<PrefetchCmd>,
    generation: Arc<AtomicU64>,
    shared: Arc<Mutex<Shared>>,
}

impl PrefetchHandle {
    /// Signal that mpv has naturally advanced to a new track.
    pub fn notify_natural_advance(&self) {
        let gen = self.generation.load(Ordering::SeqCst);
        let _ = self
            .tx
            .send(PrefetchCmd::NaturalAdvance { generation: gen });
    }

    /// Signal that the user skipped to a different track. Aborts in-flight
    /// work and schedules a new cycle.
    pub fn notify_skip(&self) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(PrefetchCmd::Skipped { generation: gen });
    }

    /// Signal that the queue was replaced or playback was stopped. Aborts
    /// in-flight work; no new cycle is scheduled until the next natural advance.
    pub fn notify_cancel(&self) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(PrefetchCmd::Cancel { generation: gen });
    }

    /// Enqueue user-requested downloads. Duplicates (already queued or
    /// currently in flight) are skipped silently. Wakes the worker so an
    /// idle cycle starts immediately.
    pub fn queue_user_downloads(&self, jobs: Vec<UserDownloadJob>) {
        if jobs.is_empty() {
            return;
        }
        {
            let mut s = self.shared.lock();
            for job in jobs {
                let already_queued = s.user_queue.iter().any(|q| q.rating_key == job.rating_key);
                let in_flight = s
                    .in_progress
                    .as_ref()
                    .is_some_and(|p| p.rating_key == job.rating_key);
                if already_queued || in_flight {
                    continue;
                }
                s.user_queue.push_back(job);
            }
        }
        // Don't bump generation — idle prefetch cycle (if any) has already
        // committed to its window. A pending generation bump would abort
        // prefetch in-flight, which is fine but wasteful. The worker's
        // run_serial_downloads picks the user queue up on its next iteration.
        let gen = self.generation.load(Ordering::SeqCst);
        let _ = self
            .tx
            .send(PrefetchCmd::UserDownloadQueued { generation: gen });
    }

    /// Cancel a queued or in-flight user download.
    pub fn cancel_user_download(&self, rating_key: &str) {
        let was_in_flight = {
            let mut s = self.shared.lock();
            s.user_queue.retain(|j| j.rating_key != rating_key);
            s.in_progress
                .as_ref()
                .is_some_and(|p| p.rating_key == rating_key)
        };
        if was_in_flight {
            let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = self
                .tx
                .send(PrefetchCmd::CancelUserDownload { generation: gen });
        }
    }

    /// Cancel every queued and in-flight user download. Always bumps the
    /// generation and sends a cancel command: even without an in-flight
    /// user download, a prefetch cycle may be mid-run and the worker needs
    /// to restart so no stale downloads-changed emission is missed.
    pub fn cancel_all_user_downloads(&self) {
        {
            let mut s = self.shared.lock();
            s.user_queue.clear();
        }
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self
            .tx
            .send(PrefetchCmd::CancelUserDownload { generation: gen });
    }

    /// Snapshot of the queue + in-flight state. Cheap — just clones a
    /// VecDeque of ratingKeys and an Option<InProgressDownload>.
    pub fn snapshot(&self) -> DownloadManagerSnapshot {
        self.shared.lock().snapshot()
    }
}

/// Spawn the long-lived prefetch / download worker task. Call once at
/// app startup. Rehydrates the ephemeral prefetch LRU cache from disk;
/// persistent downloads are rehydrated separately via
/// `rehydrate_persistent_downloads` once the cache DB is open (the DB
/// isn't available until onboarding / session restore).
pub fn spawn_worker(
    player: Arc<AudioPlayer>,
    http_client: reqwest::Client,
    app: AppHandle,
) -> PrefetchHandle {
    if let Ok(cfg_dir) = ramus_core::plex::token_store::config_dir() {
        let cache_dir = cfg_dir.join("audio_cache");
        rehydrate_cache_from_disk(&player, &cache_dir);
        // Stream-record files (subdirectory) get rehydrated AFTER the
        // primary prefetch cache — if a track has both a prefetched
        // copy and a stream-record copy, the prefetched one wins (it
        // was downloaded as a complete file via reqwest, vs the
        // stream-record which may be partial if the user skipped or
        // closed the app before track-end finalisation).
        rehydrate_stream_record_from_disk(&player, &cache_dir.join("stream_record"));
    }

    let (tx, rx) = mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(0));
    let shared = Arc::new(Mutex::new(Shared::new()));

    let worker_gen = generation.clone();
    let worker_shared = shared.clone();
    tauri::async_runtime::spawn(async move {
        worker_loop(player, http_client, app, rx, worker_gen, worker_shared).await;
    });

    PrefetchHandle {
        tx,
        generation,
        shared,
    }
}

/// Scan the audio cache directory and register files matching
/// `<rating_key>_<len>.<ext>` into the in-memory `DownloadCache`. Runs once
/// at worker startup.
fn rehydrate_cache_from_disk(player: &AudioPlayer, cache_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    let mut count: usize = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if filename.ends_with(".spec") {
            continue;
        }
        let Some((stem, _ext)) = filename.rsplit_once('.') else {
            continue;
        };
        let Some((id, len_str)) = stem.rsplit_once('_') else {
            continue;
        };
        let Ok(expected_len) = len_str.parse::<usize>() else {
            continue;
        };
        if id.len() != expected_len {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        let size = meta.len();
        if size == 0 {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        player.with_cache(|cache| {
            cache.insert(id.to_string(), path.clone(), size);
        });
        count += 1;
    }
    if count > 0 {
        log::info!("prefetch: rehydrated {count} cached track(s) from disk");
    }
}

/// Scan `<config>/audio_cache/stream_record/` for `<rating_key>.<ext>`
/// files and register them into the in-memory `DownloadCache` so that
/// `resolve_url` finds the local file:// URL on next play instead of
/// opening a fresh Plex transcode (which would overwrite the existing
/// recording with a new partial one).
///
/// Skips entries already present in the cache — the primary
/// `rehydrate_cache_from_disk` path runs first and registers any
/// prefetch-worker-downloaded files, which are guaranteed complete (no
/// libavformat tail-buffer issue). Stream-record files are a fallback
/// that may be partial; preferring the prefetched copy avoids playing
/// a song that cuts off short.
///
/// Note: a partial stream-record file (e.g. user skipped during first
/// listen, or closed the app before track-end) will rehydrate at its
/// short size and play short on next listen. The track-end re-ingest
/// path doesn't help here because file:// URLs don't trigger
/// stream-record, so the file never grows. Users wanting a complete
/// recording would need to delete the partial file from
/// `audio_cache/stream_record/` and re-play the track.
fn rehydrate_stream_record_from_disk(player: &AudioPlayer, dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut count: usize = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if filename.ends_with(".spec") {
            continue;
        }
        let Some((rating_key, _ext)) = filename.rsplit_once('.') else {
            continue;
        };
        // Skip if a prefetched copy already won the rehydration race —
        // we only fill in for tracks the prefetch cache doesn't cover.
        if player.with_cache(|c| c.get(rating_key).is_some()) {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        let size = meta.len();
        if size == 0 {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        player.with_cache(|cache| {
            cache.insert(rating_key.to_string(), path.clone(), size);
        });
        count += 1;
    }
    if count > 0 {
        log::info!("stream_record: rehydrated {count} cached recording(s) from disk");
    }
}

/// Load the `downloads` table, verify each file still exists on disk, and
/// populate `AudioPlayer::persistent_cache`. Deletes stale DB rows whose
/// files have vanished. Called once the cache DB is open — from
/// `finalize_onboarding` and from session restore in `lib.rs`.
pub fn rehydrate_persistent_downloads(
    player: &AudioPlayer,
    cache: &ramus_core::cache::db::CacheDatabase,
) {
    if let Ok(cfg_dir) = ramus_core::plex::token_store::config_dir() {
        let _ = std::fs::create_dir_all(cfg_dir.join("downloads"));
    }

    let rows = match cache.all_download_paths() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("downloads: rehydrate query failed: {e}");
            return;
        }
    };

    let mut entries: HashMap<String, PathBuf> = HashMap::new();
    let mut stale: Vec<String> = Vec::new();
    for (rating_key, file_path) in rows {
        let path = PathBuf::from(&file_path);
        // Treat zero-length files as stale: a download whose write
        // got truncated by an abrupt process suspension (or a previous
        // crash before the fsync landed) would otherwise rehydrate as
        // valid offline content and play as silence.
        let is_valid = std::fs::metadata(&path)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if is_valid {
            entries.insert(rating_key, path);
        } else {
            // Best-effort: remove the empty file so it can't accumulate.
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            stale.push(rating_key);
        }
    }

    for rk in &stale {
        let _ = cache.remove_download(rk);
    }

    let count = entries.len();
    player.rehydrate_persistent_cache(entries);
    if count > 0 {
        log::info!("downloads: rehydrated {count} permanent download(s) from disk");
    }
    if !stale.is_empty() {
        log::info!("downloads: pruned {} stale download row(s)", stale.len());
    }
}

// --- Worker loop ---

async fn worker_loop(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    mut rx: mpsc::UnboundedReceiver<PrefetchCmd>,
    shared_gen: Arc<AtomicU64>,
    shared: Arc<Mutex<Shared>>,
) {
    let mut cycle_task: Option<JoinHandle<()>> = None;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            PrefetchCmd::Cancel { .. } => {
                if let Some(h) = cycle_task.take() {
                    h.abort();
                }
                log::debug!("prefetch: cancel");
            }
            PrefetchCmd::NaturalAdvance { generation } => {
                // Let in-flight finish naturally — its loop picks up the
                // shifted window automatically. Only spawn a fresh cycle if idle.
                let idle = cycle_task.as_ref().is_none_or(|h| h.is_finished());
                if idle {
                    log::debug!("prefetch: natural advance, starting cycle gen={generation}");
                    cycle_task = Some(spawn_cycle(
                        player.clone(),
                        http.clone(),
                        app.clone(),
                        shared_gen.clone(),
                        shared.clone(),
                        generation,
                        false,
                    ));
                }
            }
            PrefetchCmd::Skipped { generation } => {
                if let Some(h) = cycle_task.take() {
                    h.abort();
                }
                log::debug!("prefetch: skip, starting cycle gen={generation}");
                cycle_task = Some(spawn_cycle(
                    player.clone(),
                    http.clone(),
                    app.clone(),
                    shared_gen.clone(),
                    shared.clone(),
                    generation,
                    true,
                ));
            }
            PrefetchCmd::UserDownloadQueued { generation } => {
                let idle = cycle_task.as_ref().is_none_or(|h| h.is_finished());
                if idle {
                    log::debug!("downloads: user queue wake, starting cycle gen={generation}");
                    cycle_task = Some(spawn_cycle(
                        player.clone(),
                        http.clone(),
                        app.clone(),
                        shared_gen.clone(),
                        shared.clone(),
                        generation,
                        false,
                    ));
                }
                // Busy: current cycle's next iteration drains user queue.
            }
            PrefetchCmd::CancelUserDownload { generation } => {
                if let Some(h) = cycle_task.take() {
                    h.abort();
                }
                // Clear any "in_progress" row so the UI doesn't show the
                // canceled item stuck mid-download. The aborted task never
                // got to clear it.
                shared.lock().in_progress = None;
                emit_downloads_changed(&app);
                log::debug!("downloads: user cancel, restarting cycle gen={generation}");
                cycle_task = Some(spawn_cycle(
                    player.clone(),
                    http.clone(),
                    app.clone(),
                    shared_gen.clone(),
                    shared.clone(),
                    generation,
                    false,
                ));
            }
        }
    }
}

fn spawn_cycle(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    shared_gen: Arc<AtomicU64>,
    shared: Arc<Mutex<Shared>>,
    my_gen: u64,
    is_skip: bool,
) -> JoinHandle<()> {
    tokio::spawn(
        async move { run_cycle(player, http, app, shared_gen, shared, my_gen, is_skip).await },
    )
}

// --- Cycle ---

/// Block until the currently-playing track's source has fully drained
/// into mpv AND the recorder has flushed its tail to disk, or until
/// `LIVE_DRAIN_CEILING` elapses, whichever comes first. Returns early
/// if the cycle is superseded.
///
/// "Drained" is a three-pronged check:
/// 1. `current_source_fully_drained()` (tight 0.25s slack) — demuxer
///    cache covers approximately the full track duration.
/// 2. `demuxer_cache_time()` hasn't advanced between consecutive polls
///    — proves mpv has actually stopped pulling, not "almost stopped".
/// 3. The stream-record file size on disk hasn't grown between polls —
///    proves the recorder's libavformat muxer has flushed its tail. The
///    bound check alone fired prematurely on slower transcodes (the
///    last 1–2% of source bytes can trickle in for several more seconds
///    after `cache_time` first crosses the threshold), leaving the
///    recorder mid-write on a page and producing torn-page files that
///    broke symphonia's Ogg probe.
///
/// All three must hold for `STABLE_POLLS` consecutive checks before we
/// declare drain. `LIVE_DRAIN_CEILING` caps total wait so a missing
/// demuxer-cache-time bridge or stuck recorder can't deadlock the
/// worker.
async fn wait_for_source_drain(
    player: &AudioPlayer,
    rating_key: Option<&str>,
    shared_gen: &Arc<AtomicU64>,
    my_gen: u64,
) {
    /// Number of consecutive polls where ALL drain signals must hold
    /// stable before we trust drain. The first poll seeds prev_*, so
    /// minimum elapsed time before drain fires is (STABLE_POLLS + 1) ×
    /// 500 ms = 2.0 s of quiet.
    const STABLE_POLLS: u32 = 3;
    /// File size must reach this fraction of the expected bytes
    /// (`duration × bitrate / 8`) before drain can fire. Opus VBR can
    /// drop more than 10% below nominal on sparse / quiet content
    /// (long fade-outs, ambient passages); 0.85 gives enough cushion
    /// that fast-Plex drain still fires on those tracks. The remaining
    /// gap to 1.0 is the slack the file_steady + cache_steady prongs
    /// have to take up to prevent premature drain mid-stream.
    const SOURCE_BYTES_FRACTION: f64 = 0.85;

    let started = Instant::now();
    let mut steady_count = 0u32;
    let mut prev_cache_time: Option<f64> = None;
    let mut prev_file_size: Option<u64> = None;
    let expected_bytes = player.expected_source_bytes_for_current();

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            return;
        }
        if started.elapsed() >= LIVE_DRAIN_CEILING {
            log::warn!(
                "stream_record: source never reported drained after {:.0}s, giving up on in-track ingest",
                LIVE_DRAIN_CEILING.as_secs_f64()
            );
            return;
        }

        let drain_met = player.current_source_fully_drained();
        let cur_cache = player.demuxer_cache_time();
        // Re-glob the file each iteration: at run_cycle entry the file
        // typically doesn't exist yet (mpv hasn't written anything).
        // `find_stream_record_file` returns None until the first byte
        // hits disk, then locks onto the same path for the rest of the
        // wait. The file-size prong stays in "skip" mode while the file
        // is absent and starts gating once we have something to size.
        let cur_file = rating_key
            .and_then(|rk| find_stream_record_file(player, rk))
            .map(|(_, len)| len);

        // Cache-time steady within 50 ms of jitter — mpv's reported
        // demuxer-cache-time can wobble fractionally even when no new
        // bytes are being pulled.
        let cache_steady = match (cur_cache, prev_cache_time) {
            (Some(c), Some(p)) => (c - p).abs() < 0.05,
            _ => false,
        };
        // File-size steadiness: skipped while the file doesn't exist
        // (caller may not have a rating_key, or mpv hasn't written
        // anything yet). Once the file exists, two consecutive Some
        // reads at the same size mean the recorder has flushed.
        let file_steady = match (cur_file, prev_file_size) {
            (None, _) => true,
            (Some(c), Some(p)) => c == p,
            (Some(_), None) => false,
        };
        // Source-completeness gate: cache_time + steadiness can both
        // hit transient lulls when Plex's chunked transcode pauses
        // mid-stream, before the body has actually finished. Compare
        // file size against `duration × bitrate / 8`; only declare
        // drain when the file has reached SOURCE_BYTES_FRACTION of
        // expected. This is what keeps the prefetch worker from opening
        // competing transcode sessions while Plex is still feeding the
        // current track — opening another session cuts the live one
        // mid-body because of Plex's per-client concurrent-transcode cap.
        // Skipped (always true) when expected_bytes is unknown
        // (missing duration or bitrate metadata) so we don't deadlock
        // on tracks without enough info.
        let bytes_met = match (expected_bytes, cur_file) {
            (Some(exp), Some(have)) => {
                have as f64 >= exp as f64 * SOURCE_BYTES_FRACTION
            }
            (None, _) => true,
            (Some(_), None) => false,
        };

        if drain_met && cache_steady && file_steady && bytes_met {
            steady_count += 1;
            if steady_count >= STABLE_POLLS {
                log::debug!(
                    "stream_record: source drained after {:.1}s (cache_time={:?}, file_size={:?}, expected_bytes={:?})",
                    started.elapsed().as_secs_f64(),
                    cur_cache,
                    cur_file,
                    expected_bytes
                );
                return;
            }
        } else {
            steady_count = 0;
        }

        prev_cache_time = cur_cache;
        prev_file_size = cur_file;
        tokio::time::sleep(LIVE_DRAIN_POLL_INTERVAL).await;
    }
}

/// One pass through the prefetch worker: wait the initial settle gap,
/// run the in-cycle stream-record ingest (which itself gates on source
/// drain), then run the serial downloads for upcoming tracks. Aborts
/// silently if the shared generation has moved on.
async fn run_cycle(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    shared_gen: Arc<AtomicU64>,
    shared: Arc<Mutex<Shared>>,
    my_gen: u64,
    is_skip: bool,
) {
    let initial_gap = if is_skip { SKIP_GAP } else { NATURAL_GAP };

    // Tiny initial sleep so mpv has issued its load request and started
    // reporting duration before we ask "is the live download done?".
    tokio::time::sleep(initial_gap).await;
    if shared_gen.load(Ordering::SeqCst) != my_gen {
        return;
    }

    // The drain wait is folded into the in-cycle ingest gate below — a
    // single pass covers both purposes: gating the stream-record analyser
    // AND gating the serial downloads that run later in this cycle.
    // A separate pre-pass was redundant and doubled the ceiling that
    // user-visible visualiser appearance has to wait through.

    let cfg_dir = match ramus_core::plex::token_store::config_dir() {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let prefetch_dir = cfg_dir.join("audio_cache");
    let downloads_dir = cfg_dir.join("downloads");
    if let Err(e) = tokio::fs::create_dir_all(&prefetch_dir).await {
        log::debug!("prefetch: cache dir create failed: {e}");
        return;
    }
    if let Err(e) = tokio::fs::create_dir_all(&downloads_dir).await {
        log::debug!("downloads: dir create failed: {e}");
        return;
    }

    let spectrum_disabled = app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum;

    // Stream-record covers both direct-play and transcoded current
    // tracks. The drain wait below guarantees mpv has fully drained the
    // source AND the recorder has flushed before we ingest, so chunked
    // Plex Ogg/Opus is just as safe to capture as direct-play FLAC/MP3.
    let try_in_cycle = !spectrum_disabled;

    // Skip the second download entirely, ingest mpv's stream-record
    // capture instead. Poll demuxer-cache-time until the source has
    // fully drained, then hand the file to the analyser + DownloadCache.
    // From that point `next_uncached_target_in_lookahead` skips the
    // current track because cache.get(rk) returns Some.
    if try_in_cycle
        && player
            .state()
            .current_track
            .as_ref()
            .is_some_and(|t| !player.with_cache(|c| c.get(&t.rating_key).is_some()))
    {
        // Pass the rating_key down so wait_for_source_drain can re-glob
        // the file each iteration: at this point mpv has only just
        // started loading and the stream-record file typically doesn't
        // exist on disk yet.
        let drain_rating_key = player.state().current_track.as_ref().map(|t| t.rating_key.clone());
        wait_for_source_drain(&player, drain_rating_key.as_deref(), &shared_gen, my_gen).await;
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            return;
        }
        if let Some(rk) = player.state().current_track.map(|t| t.rating_key) {
            try_ingest_stream_record(player.clone(), app.clone(), rk);
        }
    }

    // The in-cycle stream-record ingest above is now responsible for
    // the current track on both direct-play and transcoded paths, so
    // the serial download worker never needs to redownload it.
    let include_current = false;

    // Ensure the current + lookahead tracks that are already on disk
    // (from a previous user download or prefetch) get spectrum analysed.
    // Without this, a user who downloaded an album and pressed play sees
    // no visualiser — the prefetch worker's "download then analyse" path
    // is skipped entirely when every track is already cached, so nothing
    // else would trigger the FFT. Playing-after-download was the case the
    // user called out as still expected to analyse.
    if !spectrum_disabled {
        for (rating_key, audio_path) in player.cached_paths_in_lookahead(true) {
            if spec_file_path(&audio_path).is_file() {
                continue;
            }
            spawn_analyse_task_from_path(audio_path, rating_key, app.clone());
        }
    }

    log::debug!(
        "prefetch: serial downloads{}",
        if include_current {
            " (incl. current)"
        } else {
            ""
        },
    );
    run_serial_downloads(
        &player,
        &http,
        &app,
        &shared_gen,
        &shared,
        my_gen,
        &prefetch_dir,
        &downloads_dir,
        include_current,
    )
    .await;
}

// --- Serial download loop ---

#[allow(clippy::too_many_arguments)]
async fn run_serial_downloads(
    player: &Arc<AudioPlayer>,
    http: &reqwest::Client,
    app: &AppHandle,
    shared_gen: &Arc<AtomicU64>,
    shared: &Arc<Mutex<Shared>>,
    my_gen: u64,
    prefetch_dir: &std::path::Path,
    downloads_dir: &std::path::Path,
    include_current: bool,
) {
    let mut prefetch_failed: HashSet<String> = HashSet::new();
    let mut user_failed: HashSet<String> = HashSet::new();
    let mut consecutive_net_failures: u32 = 0;

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            log::debug!("downloads: cycle superseded, exiting");
            return;
        }

        // User queue first — always preempts prefetch.
        let user_job = {
            let mut s = shared.lock();
            loop {
                let Some(front) = s.user_queue.front().cloned() else {
                    break None;
                };
                if user_failed.contains(&front.rating_key) {
                    // Already failed this cycle — drop to avoid an infinite retry loop.
                    s.user_queue.pop_front();
                    continue;
                }
                s.user_queue.pop_front();
                break Some(front);
            }
        };

        if let Some(job) = user_job {
            match run_user_download(
                player,
                http,
                app,
                shared,
                shared_gen,
                my_gen,
                downloads_dir,
                &job,
            )
            .await
            {
                Ok(()) => {}
                Err(e) => {
                    log::warn!(
                        "downloads: user download failed for {}: {e}",
                        job.rating_key
                    );
                    emit_download_progress(
                        app,
                        progress_payload(&job, "failed", 0, job.expected_size_bytes, Some(e)),
                    );
                    user_failed.insert(job.rating_key);
                }
            }
            consecutive_net_failures = 0;
            // Loop back to pick up the next user job (or fall through to prefetch).
            continue;
        }

        // No user work — fall back to prefetch.
        let Some((track_id, url)) = player.next_uncached_target_in_lookahead(include_current)
        else {
            log::debug!("prefetch: lookahead window exhausted, idle");
            return;
        };

        if prefetch_failed.contains(&track_id) {
            log::debug!("prefetch: {track_id} already failed this cycle, ending");
            return;
        }

        match run_prefetch_download(player, http, app, prefetch_dir, &track_id, &url).await {
            Ok(()) => {
                consecutive_net_failures = 0;
                player.swap_playlist_entry_to_cached(&track_id);
                spawn_analyse_task_from_cache(player, track_id, app.clone());
            }
            Err(e) => {
                log::warn!("prefetch: serial download failed for {track_id}: {e}");
                prefetch_failed.insert(track_id);

                if is_network_error(&e) {
                    consecutive_net_failures += 1;
                    if consecutive_net_failures >= 2 {
                        log::info!(
                            "prefetch: {} consecutive network failures, triggering connection re-evaluation",
                            consecutive_net_failures,
                        );
                        let monitor = app
                            .state::<crate::state::AppState>()
                            .connection_monitor
                            .clone();
                        tokio::spawn(async move {
                            monitor.evaluate_connection().await;
                        });
                        return;
                    }
                } else {
                    consecutive_net_failures = 0;
                }
            }
        }
    }
}

fn is_network_error(err: &str) -> bool {
    err.contains("request error")
        || err.contains("timed out")
        || err.contains("connection refused")
        || err.contains("connection reset")
}

// --- Prefetch downloads (ephemeral, go into LRU DownloadCache) ---

async fn run_prefetch_download(
    player: &AudioPlayer,
    client: &reqwest::Client,
    _app: &AppHandle,
    cache_dir: &std::path::Path,
    track_id: &str,
    url: &str,
) -> Result<(), String> {
    if player.with_cache(|c| c.get(track_id).is_some()) {
        return Ok(());
    }
    if player.has_persistent_download(track_id) {
        // Already permanently downloaded; no prefetch needed.
        return Ok(());
    }

    let ext = extension_from_url(url);
    let filename = format!("{}_{}.{}", sanitize_filename(track_id), track_id.len(), ext);
    let file_path = cache_dir.join(&filename);

    let size = download_http_to_file(client, url, &file_path, |_bytes, _total| {}).await?;

    let current_id = player.current_track_id();
    let evicted = player.with_cache(|cache| {
        cache.insert(track_id.to_string(), file_path.clone(), size);
        cache.evict_if_needed(current_id.as_deref())
    });
    for path in evicted {
        let spec = spec_file_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&spec).await;
    }

    log::debug!("prefetch: cached {track_id} ({size} bytes)");
    Ok(())
}

// --- User downloads (persistent, go into `downloads` table) ---

#[allow(clippy::too_many_arguments)]
async fn run_user_download(
    player: &AudioPlayer,
    client: &reqwest::Client,
    app: &AppHandle,
    shared: &Arc<Mutex<Shared>>,
    shared_gen: &Arc<AtomicU64>,
    my_gen: u64,
    downloads_dir: &std::path::Path,
    job: &UserDownloadJob,
) -> Result<(), String> {
    if player.has_persistent_download(&job.rating_key) {
        // Already done — still emit a terminal event so the UI clears the row.
        emit_download_progress(
            app,
            progress_payload(
                job,
                "done",
                job.expected_size_bytes.unwrap_or(0),
                job.expected_size_bytes,
                None,
            ),
        );
        emit_downloads_changed(app);
        return Ok(());
    }

    let ext = {
        let codec_ext = job.codec.to_lowercase();
        if is_allowed_extension(&codec_ext) {
            codec_ext
        } else {
            extension_from_url(&job.url)
        }
    };
    // sanitize_filename whitelists [a-zA-Z0-9_-]; a ratingKey composed
    // entirely of stripped characters (e.g. "../" or "/") would produce
    // an empty stem and the filename `.{ext}` — a hidden dotfile that
    // every such track would collide on. Guard against it explicitly so
    // we surface the error instead of silently corrupting downloads.
    let stem = sanitize_filename(&job.rating_key);
    if stem.is_empty() {
        return Err(format!(
            "download rejected: ratingKey {:?} has no filesystem-safe characters",
            job.rating_key
        ));
    }
    let filename = format!("{stem}.{ext}");
    let file_path = downloads_dir.join(&filename);

    // Mark in-flight and emit a zero-byte "downloading" start event.
    shared.lock().in_progress = Some(InProgressDownload {
        rating_key: job.rating_key.clone(),
        album_rating_key: job.album_rating_key.clone(),
        title: job.title.clone(),
        artist_name: job.artist_name.clone(),
        album_title: job.album_title.clone(),
        thumb: job.thumb.clone(),
        bytes_written: 0,
        total_bytes: job.expected_size_bytes,
    });
    emit_download_progress(
        app,
        progress_payload(job, "downloading", 0, job.expected_size_bytes, None),
    );

    let rk = job.rating_key.clone();
    let job_for_cb = job.clone();
    let app_for_cb = app.clone();
    let shared_for_cb = shared.clone();
    let expected = job.expected_size_bytes;
    let mut last_emit = Instant::now();

    let download_result =
        download_http_to_file(client, &job.url, &file_path, move |bytes, total| {
            let total = total.or(expected);
            {
                let mut s = shared_for_cb.lock();
                if let Some(ip) = s.in_progress.as_mut() {
                    if ip.rating_key == rk {
                        ip.bytes_written = bytes;
                        ip.total_bytes = total;
                    }
                }
            }
            if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                last_emit = Instant::now();
                emit_download_progress(
                    &app_for_cb,
                    progress_payload(&job_for_cb, "downloading", bytes, total, None),
                );
            }
        })
        .await;

    // Regardless of outcome, clear the in-flight slot.
    shared.lock().in_progress = None;

    // Check for a late cancellation (generation bumped while we were in-flight).
    if shared_gen.load(Ordering::SeqCst) != my_gen {
        let _ = tokio::fs::remove_file(&file_path).await;
        emit_downloads_changed(app);
        return Err("canceled".into());
    }

    let size = download_result?;

    // Persist to the downloads table.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let row = DownloadRow {
        rating_key: job.rating_key.clone(),
        album_rating_key: job.album_rating_key.clone(),
        file_path: file_path.to_string_lossy().into_owned(),
        size_bytes: size as i64,
        codec: job.codec.clone(),
        downloaded_at: now,
    };
    // Persist the DB row BEFORE pinning the file in memory or registering
    // it for local-first playback. If the DB write fails, the file would
    // otherwise stay on disk forever (no rehydrate row on next launch) and
    // the in-memory pin would lie about offline availability for one
    // session. Fail hard and clean up.
    let insert_result = {
        let state = app.state::<crate::state::AppState>();
        let cache_guard = state.cache.lock();
        match cache_guard.as_ref() {
            Some(cache) => cache.insert_download(&row).map_err(|e| e.to_string()),
            None => Err("downloads cache not ready".to_string()),
        }
    };
    if let Err(e) = insert_result {
        log::error!("downloads: insert_download failed, removing partial file: {e}");
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err(format!("download persist failed: {e}"));
    }

    // Register for local-first playback and skip iOS backup.
    player.register_persistent_download(job.rating_key.clone(), file_path.clone());
    ios_backup::exclude_from_backup(&file_path);

    // Warm the ancillary caches so offline playback has everything the UI
    // needs: waveform sidecar for the seek bar, and album art pre-fetched
    // at every display size. Fire-and-forget — if the network drops
    // between audio download and these best-effort fetches, we just degrade
    // gracefully at render time.
    //
    // The user can remove the download between this point and the spawn
    // running. We re-check `has_persistent_download` at the top to skip
    // wasted bandwidth, then call `recompute_image_pins` after warming as
    // a backstop in case removal lands mid-warm.
    {
        let app_warm = app.clone();
        let rk = job.rating_key.clone();
        let thumb = job.thumb.clone();
        let file_path_warm = file_path.clone();
        tauri::async_runtime::spawn(async move {
            let state = app_warm.state::<crate::state::AppState>();
            if !state.player.has_persistent_download(&rk) {
                return;
            }
            crate::commands::downloads::warm_waveform_sidecar(&state.client, &rk, &file_path_warm)
                .await;
            if let Some(thumb) = thumb {
                crate::commands::downloads::warm_art_cache(
                    &state.image_cache,
                    &state.client,
                    &state.http_client,
                    &thumb,
                )
                .await;
                crate::commands::downloads::recompute_image_pins(&state);
            }
        });
    }

    // If the downloaded track sits in the current playback queue, swap its
    // mpv playlist entry to the local file so the next time we hit that
    // track we read from disk. swap_playlist_entry_to_cached will kick
    // off spectrum analysis for the track when it becomes the next
    // prefetch candidate — we intentionally DON'T analyse at download
    // time because bulk starred-downloads would then FFT hundreds of
    // tracks the user isn't about to play.

    player.swap_playlist_entry_to_cached(&job.rating_key);

    emit_download_progress(app, progress_payload(job, "done", size, Some(size), None));
    emit_downloads_changed(app);

    log::info!("downloads: stored {} ({size} bytes)", job.rating_key);
    Ok(())
}

// --- Spectrum analysis ---

/// Hand off a `stream-record`-captured file (produced by mpv during
/// playback) to the spectrum analyser, and register it in the prefetch
/// `DownloadCache` so subsequent `resolve_url` calls for this rating-key
/// pick up the local file:// path instead of opening another HTTP fetch.
///
/// Idempotent: bails early if the rating-key is already in DownloadCache
/// (so a double-fire doesn't double-insert), or if the file is too small
/// to be worth analysing (mpv may write a partial file if the user
/// skipped before the source could drain).
///
/// Spawns a tokio task internally so the caller can return immediately
/// — the task waits for the file to be byte-stable before firing the
/// analyser. mpv's recorder flushes pages asymchronously: even after
/// `wait_for_source_drain` reports the demuxer cache covers the full
/// duration, the recorder may still be writing the tail few KB. Probing
/// during that mid-page-write produces `UnexpectedEof` from symphonia's
/// strict Ogg parser. Polling at 250 ms until the size doesn't grow for
/// two consecutive checks is enough to land on a coherent page boundary
/// without waiting for full file finalisation (which only happens on
/// playlist transition).
/// Locate the stream-record file produced by mpv for the given
/// rating-key. Files are named `<rating_key>.<ext>` and we don't know
/// the extension ahead of time, so glob by prefix. Returns the largest
/// match (in case of stale leftovers from a prior session) along with
/// its size, or `None` if no file exists yet / `stream_record_dir` is
/// unset.
pub fn find_stream_record_file(
    player: &AudioPlayer,
    rating_key: &str,
) -> Option<(PathBuf, u64)> {
    let dir = player.stream_record_dir()?;
    let prefix = format!("{rating_key}.");
    let mut best: Option<(PathBuf, u64)> = None;
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(_, prev_len)| len > *prev_len) {
            best = Some((p, len));
        }
    }
    best
}

pub fn try_ingest_stream_record(
    player: Arc<AudioPlayer>,
    app: AppHandle,
    rating_key: String,
) {
    log::debug!("stream_record: try_ingest invoked for rating_key={rating_key}");
    let Some((path, initial_size)) = find_stream_record_file(&player, &rating_key) else {
        log::debug!(
            "stream_record: no file found for rating_key={rating_key} — bailing (stream_record_dir set? {})",
            player.stream_record_dir().is_some()
        );
        return;
    };
    // 32 KiB is generous — even a 5-second 96 kbps Opus snippet runs ~60
    // KiB. Anything below that is mpv writing a header for a track the
    // user skipped immediately.
    if initial_size < 32_768 {
        log::debug!(
            "stream_record: skip ingest of {path:?} ({initial_size} bytes) — too small to analyse"
        );
        return;
    }
    let prev_cached_size = player.with_cache(|c| c.size(&rating_key));
    log::debug!(
        "stream_record: queued ingest of {path:?} ({initial_size} bytes, prev_cached_size={prev_cached_size:?}) for rating_key={rating_key}, awaiting byte-stability"
    );

    // tauri::async_runtime::spawn (NOT tokio::spawn) because this
    // function is called from the mpv event-loop thread via the
    // on_playlist_pos_change callback, which isn't itself a tokio
    // runtime context. `tokio::spawn` panics with "no reactor running"
    // there. tauri's async runtime wrapper picks up the right handle
    // regardless of caller thread.
    tauri::async_runtime::spawn(async move {
        // Poll metadata until size is stable for two consecutive ticks
        // (= ~500 ms of quiet). MAX_POLLS bounds the wait when the
        // recorder is actively writing — for an actively-growing
        // stream-record file (in-cycle ingest after a ceiling-fire),
        // size keeps changing, stable_count keeps resetting, and we'd
        // wait the full bound before proceeding. The bounded Ogg
        // reader in `analyse_file` clamps reads to the last complete
        // page boundary, so analysing a file mid-write is safe even
        // without strict stability — keep the bound short so a
        // visualiser appears promptly.
        //
        // The stability poll runs BEFORE the growth check against the
        // cached size: at the moment on_playlist_pos_change fires for
        // the next track, mpv's recorder for the previous track is
        // closing but libavformat may not have finished flushing yet.
        // `initial_size` could equal the in-cycle ingest's cached size
        // even though the file is about to grow by ~10s of audio.
        // Polling first lets the flush complete; then the growth check
        // sees the real final_size.
        const POLL_INTERVAL: Duration = Duration::from_millis(250);
        const MAX_POLLS: u32 = 8; // 2 s upper bound
        const STABLE_THRESHOLD: u32 = 2;
        let mut last_size = initial_size;
        let mut stable_count = 0u32;
        let mut total_waited_ms = 0u64;
        for _ in 0..MAX_POLLS {
            tokio::time::sleep(POLL_INTERVAL).await;
            total_waited_ms += POLL_INTERVAL.as_millis() as u64;
            let cur = match std::fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(e) => {
                    log::warn!("stream_record: stat({path:?}) failed during stability poll: {e}");
                    return;
                }
            };
            if cur == last_size {
                stable_count += 1;
                if stable_count >= STABLE_THRESHOLD {
                    log::debug!(
                        "stream_record: file {path:?} stabilised at {cur} bytes after {total_waited_ms} ms"
                    );
                    break;
                }
            } else {
                stable_count = 0;
                last_size = cur;
            }
        }
        let final_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(last_size);

        // Already cached and the file hasn't grown meaningfully since
        // first analysis: skip the redundant work. 64 KiB is well
        // beyond Plex VBR jitter but tighter than a meaningful tail
        // flush.
        if let Some(prev) = prev_cached_size {
            const REGROW_THRESHOLD: u64 = 64 * 1024;
            if final_size < prev + REGROW_THRESHOLD {
                log::info!(
                    "stream_record: file {path:?} stable at {final_size} bytes, no growth since cached size {prev} — skipping re-analysis"
                );
                return;
            }
            log::info!(
                "stream_record: file grew from {prev} to {final_size} bytes since last ingest, re-analysing"
            );
        }

        log::debug!(
            "stream_record: ingesting {path:?} ({final_size} bytes) for rating_key={rating_key}"
        );
        player.with_cache(|c| c.insert(rating_key.clone(), path.clone(), final_size));
        // Rewrite the mpv playlist entry to a file:// URL pointing at
        // the recorder file. Without this, when the user skips back
        // to this track mpv reloads the original network transcode URL
        // (which is still in mpv's playlist memory) and starts a fresh
        // transcode session, OVERWRITING the recorder file from byte 0
        // and producing a new partial spec — undoing all the work we
        // just did. `swap_playlist_entry_to_cached` no-ops when the
        // track is the currently-playing entry (the `idx == queue_index`
        // guard inside), so calling it from the in-cycle ingest path
        // (where the track IS still playing) is harmless; it only
        // takes effect for the track-end re-ingest path (where the
        // track has just transitioned away).
        player.swap_playlist_entry_to_cached(&rating_key);
        // Force re-analysis: a previous in-cycle ingest may have
        // written a partial spec on disk; without forcing, the
        // analyser would short-circuit on the existing spec and never
        // process the now-grown audio file.
        spawn_analyse_task_force(path, rating_key, app);
    });
}

fn spawn_analyse_task_from_cache(player: &AudioPlayer, track_id: String, app: AppHandle) {
    if app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum
    {
        return;
    }
    let Some(audio_path) = player.with_cache(|c| c.get(&track_id).map(|p| p.to_path_buf())) else {
        return;
    };
    spawn_analyse_task_from_path(audio_path, track_id, app);
}

fn spawn_analyse_task_from_path(audio_path: PathBuf, track_id: String, app: AppHandle) {
    spawn_analyse_task_from_path_inner(audio_path, track_id, app, false);
}

/// Force-re-analyse variant. Used by the stream-record track-end
/// re-ingest path to overwrite a partial in-cycle spec with a fresh
/// one covering the now-finalised audio file. Without forcing, the
/// existing partial spec would short-circuit `read_spec_file is_some`
/// and the analyser would never run on the grown file.
fn spawn_analyse_task_force(audio_path: PathBuf, track_id: String, app: AppHandle) {
    spawn_analyse_task_from_path_inner(audio_path, track_id, app, true);
}

fn spawn_analyse_task_from_path_inner(
    audio_path: PathBuf,
    track_id: String,
    app: AppHandle,
    force: bool,
) {
    if app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum
    {
        return;
    }
    if !force && read_spec_file(&audio_path).is_some() {
        emit_spectrum_ready(&app, track_id);
        return;
    }
    // tauri::async_runtime::spawn_blocking instead of tokio's: the
    // call chain reaches here both from tokio-runtime contexts (the
    // prefetch worker) and from spawned tasks under the tauri runtime
    // wrapper. The tauri variant resolves to the right handle in
    // either case.
    tauri::async_runtime::spawn_blocking(move || {
        spectrum_analyzer::analyse_and_persist(&audio_path);
        emit_spectrum_ready(&app, track_id);
    });
}

// --- Shared HTTP download core ---

fn extension_from_url(url: &str) -> String {
    // Single-file transcode URLs have no extension on the path
    // (`/audio/:/transcode/universal/start`) but always return Ogg/Opus
    // bytes — see `build_transcode_download_url`.
    if ramus_core::playback::transcode::is_transcode_download_url(url) {
        return "ogg".to_string();
    }
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.path().rsplit('.').next().map(|e| e.to_lowercase()))
        .filter(|e| is_allowed_extension(e))
        .unwrap_or_else(|| "bin".to_string())
}

/// Shared resumable HTTP download routine. Writes `url` into `file_path`
/// with Range-resume retries, a 90s budget, and adaptive backoff. Calls
/// `on_progress(bytes_written, expected_total_bytes)` as bytes land. Returns
/// the final file size on success.
///
/// Cancellation is external — callers wrap this in `tokio::spawn` and
/// abort the task if needed. Partial files survive abort so the next call
/// with the same destination path can resume.
/// reqwest's `Error::Display` impl prefixes "for url (...)" with the
/// full request URL, which carries `X-Plex-Token` in the query string.
/// Use the inner source's message instead, falling back to a category
/// label, so logs never echo the token back.
fn redact_reqwest_err(e: &reqwest::Error) -> String {
    if let Some(src) = std::error::Error::source(e) {
        return src.to_string();
    }
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        "connect".into()
    } else if let Some(status) = e.status() {
        format!("status={status}")
    } else {
        "request error".into()
    }
}

async fn download_http_to_file(
    client: &reqwest::Client,
    url: &str,
    file_path: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<u64, String> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(file_path)
        .await
        .map_err(|e| format!("create file: {e}"))?;
    // Set NSURLIsExcludedFromBackupKey as soon as the file exists, not
    // only after a successful download. A cancelled or interrupted
    // partial sitting in downloads/ would otherwise be eligible for
    // iCloud backup until the next retry succeeds.
    ios_backup::exclude_from_backup(file_path);
    // Resume offset is whatever the OPENED file's end is. Querying
    // tokio::fs::metadata before opening would race: another task could
    // truncate the partial between the two syscalls, leaving `written`
    // larger than the actual file and producing a sparse zero-padded gap
    // when subsequent write_all calls land at the stale offset.
    let mut written: u64 = file
        .seek(std::io::SeekFrom::End(0))
        .await
        .map_err(|e| format!("seek end: {e}"))?;

    let mut expected_size: Option<u64> = None;
    let mut retries: u32 = 0;
    let download_start = Instant::now();
    let deadline = download_start + DOWNLOAD_TIME_BUDGET;
    let mut current_backoff = INITIAL_BACKOFF;

    loop {
        let written_before_attempt = written;

        let mut request = client.get(url);
        if written > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={written}-"));
        }

        let mut response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                retries += 1;
                let cause = redact_reqwest_err(&e);
                if Instant::now() >= deadline {
                    return Err(format!("request error after {retries} retries: {cause}"));
                }
                log::debug!("download: request error (attempt {retries}): {cause}");
                current_backoff = (current_backoff * 2).min(MAX_BACKOFF);
                tokio::time::sleep(current_backoff).await;
                continue;
            }
        };

        let status = response.status();

        if written == 0 || expected_size.is_none() {
            let cl = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(none)")
                .to_string();
            log::debug!("download: {status}, content-length={cl}");
            expected_size = cl.parse().ok().map(|cl: u64| cl + written);
            on_progress(written, expected_size);
        }

        // 416 Range Not Satisfiable: stale/complete partial on disk.
        if status.as_u16() == 416 {
            drop(file);
            let _ = tokio::fs::remove_file(file_path).await;
            return Err(format!(
                "HTTP 416 — stale partial at {written} bytes removed, will retry next cycle"
            ));
        }

        if !status.is_success() && status.as_u16() != 206 {
            return Err(format!("HTTP {status}"));
        }

        // Server ignored the Range header — start over from scratch.
        if written > 0 && status.as_u16() == 200 {
            written = 0;
            expected_size = None;
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|e| e.to_string())?;
            file.set_len(0).await.map_err(|e| e.to_string())?;
            on_progress(written, expected_size);
        }

        let mut chunk_error = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| format!("write error: {e}"))?;
                    written += chunk.len() as u64;
                    if written > MAX_DOWNLOAD_BYTES {
                        return Err(format!(
                            "download exceeded {MAX_DOWNLOAD_BYTES}-byte cap at {written} bytes"
                        ));
                    }
                    on_progress(written, expected_size);
                }
                Ok(None) => break,
                Err(e) => {
                    log::debug!(
                        "download: chunk error at {written} bytes: {}",
                        redact_reqwest_err(&e),
                    );
                    if let Some(expected) = expected_size {
                        if written >= expected {
                            break;
                        }
                    }
                    chunk_error = true;
                    break;
                }
            }
        }

        if let Some(expected) = expected_size {
            if written >= expected {
                break;
            }
        } else if !chunk_error {
            break;
        }

        retries += 1;
        let now = Instant::now();
        // saturating_sub guards against the resume-was-rejected case: if
        // the server returned 200 on a Range request earlier in this
        // iteration, `written` got reset to 0 while `written_before_attempt`
        // is still the pre-reset value. We lost progress this attempt, so
        // report 0 gained rather than underflowing.
        let bytes_this_attempt = written.saturating_sub(written_before_attempt);

        if now >= deadline {
            return Err(format!(
                "gave up after {retries} retries ({:.1}s): got {written} of {} bytes",
                (now - download_start).as_secs_f64(),
                expected_size
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
        }

        if bytes_this_attempt >= MIN_PROGRESS_BYTES {
            current_backoff = INITIAL_BACKOFF;
        } else {
            current_backoff = (current_backoff * 2).min(MAX_BACKOFF);
        }

        let remaining = deadline.saturating_duration_since(now);
        log::debug!(
            "download: resuming at {written}/{} \
             (attempt {retries}, +{}B, backoff {}ms, {:.0}s left)",
            expected_size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into()),
            bytes_this_attempt,
            current_backoff.as_millis(),
            remaining.as_secs_f64(),
        );

        tokio::time::sleep(current_backoff).await;
    }

    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    // sync_all forces the kernel to durably write the data + metadata
    // before we report success. Without this, an iOS/Android process
    // suspension immediately after flush could leave the file zero-length
    // on disk while the DB row + persistent_cache pin claim it's complete.
    file.sync_all().await.map_err(|e| format!("sync: {e}"))?;
    Ok(written)
}
