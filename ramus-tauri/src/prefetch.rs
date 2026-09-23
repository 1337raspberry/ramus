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

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use ramus_core::cache::downloads::DownloadRow;
use ramus_core::playback::player::{is_allowed_extension, sanitize_filename, AudioPlayer};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::{
    emit_download_progress, emit_downloads_changed, emit_metadata_warmed, DownloadProgressPayload,
    MetadataWarmedPayload,
};
use crate::ios_backup;

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

/// Soft ceiling on the live-drain wait. Plex transcodes that arrive
/// faster than realtime drain in 5–15s and exit before this. Past the
/// ceiling the wait gives up once the source has also stopped advancing
/// (debounced over `STABLE_POLLS`). While the source is still delivering
/// (a realtime-paced live transcode) the wait extends toward the hard
/// ceiling, holding back the serial downloads whose competing transcode
/// session would cut the live stream.
const LIVE_DRAIN_CEILING: Duration = Duration::from_secs(30);

/// Absolute cap on the live-drain wait, used only while the source is still
/// actively feeding mpv (so the soft `LIVE_DRAIN_CEILING` is being extended to
/// avoid cutting a realtime-paced live transcode). Normally the wait is bounded
/// by the track's own duration (`dur + slack`); this is the fallback when the
/// track duration is unknown, so a stuck source can't hang the worker forever.
const HARD_LIVE_DRAIN_CEILING: Duration = Duration::from_secs(600);

/// Poll interval for the live-drain wait. Cheap (a single mpv property
/// read per tick), so a tight cadence makes the post-drain prefetch
/// fire promptly.
const LIVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How long to hold a speculative download back while the live stream is
/// starving. Short enough to make use of the good windows between stalls on
/// a flaky link, long enough that the re-check costs nothing.
const STARVED_BACKOFF: Duration = Duration::from_secs(5);

/// Emit progress events at most this often per in-flight download. Bytes
/// ticks at chunk rate (>50Hz on LAN); throttling keeps the event bus quiet.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(250);

/// Whether a mid-download progress tick should go out: throttled to
/// [`PROGRESS_EMIT_INTERVAL`], and never to a hidden webview (each emit would
/// wake a suspended one; the next tick after it reappears is at most one
/// interval away). `webview_hidden` is only read once a tick is due. The
/// start, done and failed events are not ticks and always go out.
fn progress_tick_due(since_last: Duration, webview_hidden: impl FnOnce() -> bool) -> bool {
    since_last >= PROGRESS_EMIT_INTERVAL && !webview_hidden()
}

/// Whether a cycle's next unit of work must first let the live track's
/// network source drain. Asked before every unit rather than once at the
/// start: a cycle that began with nothing streaming (a restored queue not
/// yet handed to mpv) must still wait once playback starts, or the rest of
/// its downloads would run alongside the stream mpv has just opened. A
/// cycle waits at most once.
#[derive(Debug, Default)]
struct DrainGate {
    waited: bool,
}

impl DrainGate {
    /// `streams_from_network` is only read while the cycle has not waited.
    fn due(&self, streams_from_network: impl FnOnce() -> bool) -> bool {
        !self.waited && streams_from_network()
    }

    fn drained(&mut self) {
        self.waited = true;
    }
}

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
    // Defense-in-depth: every current caller passes an already-redacted
    // string (via redact_reqwest_err), but the renderer surfaces this
    // verbatim in download toasts and the debug panel — wrap once more
    // here so a future error path that forgets can't leak a token.
    let error = error.map(|e| ramus_core::util::redact_urls(&e));
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
    /// Track length in seconds. Carried so the download-time lyrics warm can
    /// query LRCLIB, which matches on duration.
    pub duration: f64,
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
        rehydrate_cache_from_disk(&player, &cfg_dir.join("audio_cache"));
        tauri::async_runtime::spawn_blocking(move || purge_legacy_spectrum_files(&cfg_dir));
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
        // Sidecars (`.wave`, `.lyrics`) and in-flight `.part` files fail
        // this parse: their stem still carries the audio extension, so the
        // length field never parses.
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

/// Remove what the retired precomputed visualiser left on disk: the
/// `audio_cache/stream_record/` capture directory and any `.spec`
/// spectrogram sidecars next to cached or downloaded audio. Runs once at
/// worker startup and is a no-op once the files are gone.
fn purge_legacy_spectrum_files(cfg_dir: &Path) {
    let record_dir = cfg_dir.join("audio_cache").join("stream_record");
    if record_dir.is_dir() {
        match std::fs::remove_dir_all(&record_dir) {
            Ok(()) => log::info!("prefetch: removed legacy stream_record directory"),
            Err(e) => log::warn!("prefetch: failed to remove {record_dir:?}: {e}"),
        }
    }
    let mut removed = 0usize;
    for dir in ["audio_cache", "downloads"] {
        let Ok(entries) = std::fs::read_dir(cfg_dir.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "spec")
                && path.is_file()
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        log::info!("prefetch: removed {removed} legacy .spec sidecar(s)");
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
/// into mpv, or until the drain ceiling elapses. Returns early if the
/// cycle is superseded.
///
/// "Drained" is a two-pronged check:
/// 1. `current_source_fully_drained()` (tight 0.25s slack) — demuxer
///    cache covers approximately the full track duration.
/// 2. `demuxer_cache_time()` hasn't advanced between consecutive polls
///    — proves mpv has actually stopped pulling, not "almost stopped".
///
/// Both must hold for `STABLE_POLLS` consecutive checks before we
/// declare drain. The wait gives up at `LIVE_DRAIN_CEILING` only if the
/// source has stopped advancing by then; while demuxer-cache-time is still
/// climbing (a realtime-paced live transcode actively feeding), it extends
/// up to a hard ceiling tied to the track length so it doesn't open a
/// competing prefetch session that would cut the live one. The hard ceiling
/// (or `HARD_LIVE_DRAIN_CEILING` when the duration is unknown) is the
/// anti-hang backstop for a missing demuxer-cache-time bridge.
async fn wait_for_source_drain(player: &AudioPlayer, shared_gen: &Arc<AtomicU64>, my_gen: u64) {
    /// Number of consecutive polls where both drain signals must hold
    /// stable before we trust drain. The first poll seeds prev_cache_time,
    /// so the minimum elapsed time before drain fires is
    /// (STABLE_POLLS + 1) × 500 ms = 2.0 s of quiet.
    const STABLE_POLLS: u32 = 3;

    let started = Instant::now();
    let mut steady_count = 0u32;
    let mut not_advancing_polls = 0u32;
    let mut prev_cache_time: Option<f64> = None;

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            return;
        }
        let drain_met = player.current_source_fully_drained();
        let cur_cache = player.demuxer_cache_time();

        // While the source is still actively feeding mpv (demuxer-cache-time
        // advancing), extend the wait past the soft ceiling rather than open a
        // competing prefetch session that would cut a realtime-paced live
        // transcode (Plex's ~1-transcoder cap). Give up only once the source
        // has stopped advancing (fully arrived or stalled) past the soft
        // ceiling, or a hard ceiling tied to the track length elapses — the
        // anti-hang backstop when demuxer-cache-time is unavailable.
        let cache_advancing =
            matches!((cur_cache, prev_cache_time), (Some(c), Some(p)) if c - p > 0.05);
        // Debounce "stopped advancing" the same way drain is debounced:
        // Plex's chunked transcode delivery is bursty, and a single >500ms
        // chunk gap on a flaky link must not collapse the past-ceiling
        // extension — opening a competing session mid-body cuts the live
        // transcode (the exact stall this extension exists to prevent).
        if cache_advancing {
            not_advancing_polls = 0;
        } else {
            not_advancing_polls += 1;
        }
        let hard_ceiling = {
            let dur = player.duration();
            if dur > 0.0 {
                Duration::from_secs_f64(dur + 15.0).max(LIVE_DRAIN_CEILING)
            } else {
                HARD_LIVE_DRAIN_CEILING
            }
        };
        if started.elapsed() >= hard_ceiling
            || (started.elapsed() >= LIVE_DRAIN_CEILING && not_advancing_polls >= STABLE_POLLS)
        {
            log::warn!(
                "prefetch: source not drained after {:.0}s (cache_advancing={cache_advancing}), proceeding",
                started.elapsed().as_secs_f64()
            );
            return;
        }

        // Cache-time steady within 50 ms of jitter — mpv's reported
        // demuxer-cache-time can wobble fractionally even when no new
        // bytes are being pulled.
        let cache_steady = match (cur_cache, prev_cache_time) {
            (Some(c), Some(p)) => (c - p).abs() < 0.05,
            _ => false,
        };

        log::debug!(
            "prefetch: drain probe t={:.1}s drain_met={drain_met} cache_steady={cache_steady} cache_time={cur_cache:?} steady_count={steady_count}",
            started.elapsed().as_secs_f64(),
        );

        if drain_met && cache_steady {
            steady_count += 1;
            if steady_count >= STABLE_POLLS {
                log::debug!(
                    "prefetch: source drained after {:.1}s (cache_time={cur_cache:?})",
                    started.elapsed().as_secs_f64(),
                );
                return;
            }
        } else {
            steady_count = 0;
        }

        prev_cache_time = cur_cache;
        tokio::time::sleep(LIVE_DRAIN_POLL_INTERVAL).await;
    }
}

/// One pass through the prefetch worker: wait the initial settle gap, then
/// run the serial downloads for upcoming tracks, held until the live source
/// has drained. Aborts silently if the shared generation has moved on.
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

    log::debug!("prefetch: serial downloads");
    run_serial_downloads(
        &player,
        &http,
        &app,
        &shared_gen,
        &shared,
        my_gen,
        &prefetch_dir,
        &downloads_dir,
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
) {
    let mut prefetch_failed: HashSet<String> = HashSet::new();
    let mut user_failed: HashSet<String> = HashSet::new();
    let mut warm_failed: HashSet<String> = HashSet::new();
    let mut consecutive_net_failures: u32 = 0;
    let mut drain = DrainGate::default();

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            log::debug!("downloads: cycle superseded, exiting");
            return;
        }

        // Hold downloads until the live track has stopped pulling from its
        // source. Opening a competing transcode session while Plex is still
        // feeding the current track cuts the live one (Plex's ~1-transcoder
        // cap) — the cause of "9/9 cached but song 1 keeps stalling" on a slow
        // link. Runs on every platform. A local file or an unmaterialised
        // restored queue has nothing streaming, so waiting for a "live source"
        // to drain would just burn the ceiling; `DrainGate` asks again before
        // each unit so playback starting mid-cycle is still waited for.
        if drain.due(|| player.current_track_streams_from_network()) {
            wait_for_source_drain(player, shared_gen, my_gen).await;
            drain.drained();
            continue;
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

        // No user work — fall back to prefetch audio.
        let Some((track_id, url)) = player.next_uncached_target_in_lookahead() else {
            // All in-window audio is cached. Spend idle time on the
            // lowest-priority warming tier — waveform sidecars and 1200px
            // hero art for tracks that are already playable offline. One
            // bounded unit per loop, then fall through to the top so a
            // fresh audio target, a queued user download, or a skip
            // preempts before the next unit runs.
            match next_warm_unit(app, player, &warm_failed) {
                Some(unit) => {
                    if run_warm_unit(app, &unit).await {
                        // Tell the UI the artefact landed — surfaces whose
                        // original fetch failed (or ran before the warm)
                        // re-request instead of showing a placeholder until
                        // the next track change.
                        emit_metadata_warmed(app, unit.warmed_payload());
                    } else {
                        warm_failed.insert(unit.failed_key());
                    }
                    continue;
                }
                None => {
                    log::debug!("prefetch: lookahead window fully cached and warmed, idle");
                    return;
                }
            }
        };

        if prefetch_failed.contains(&track_id) {
            log::debug!("prefetch: {track_id} already failed this cycle, ending");
            return;
        }

        // The drain gate above only holds the line until it gives up (a slow
        // link's ordinary stalls satisfy its "source stopped advancing" exit
        // once the soft ceiling passes), after which nothing re-checks live
        // health for the rest of the cycle. Speculative downloads would then
        // spend the remainder of the track competing for bandwidth the live
        // stream is already short of.
        //
        // Deliberately a back-off, never a stop: getting the queue onto disk
        // is what actually fixes a slow link, so the worker keeps trying and
        // uses the good windows between stalls. Looping back to the top rather
        // than sleeping in place keeps a user download, a skip, or a fresh
        // target able to preempt, and re-resolves the URL under current policy.
        if player.is_starving() {
            log::debug!("prefetch: live stream starving, holding {track_id} back");
            tokio::time::sleep(STARVED_BACKOFF).await;
            continue;
        }

        match run_prefetch_download(player, http, app, prefetch_dir, &track_id, &url).await {
            Ok(()) => {
                consecutive_net_failures = 0;
                player.swap_playlist_entry_to_cached(&track_id);
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
        || err.contains("timeout")
        || err.contains("connection refused")
        || err.contains("connection reset")
        // The downloader's time-budget exhaustion ("gave up after N retries")
        // is the shape a dead or black-holed connection takes when every
        // resume attempt inside the budget fails — it must count toward the
        // consecutive-failure trigger, not reset it, or a fully dead network
        // never reaches the connection re-evaluation this gate exists for.
        || err.contains("gave up after")
}

// --- Lowest-priority cache warming (waveform sidecars + hero art) ---

/// The now-playing hero renders album art at this size; it's the only art
/// size never pre-warmed by the UI (the up-next list greedily fetches 72px),
/// so offline playback otherwise falls back to the blurry thumb.
const HERO_ART_SIZE: u32 = 1200;

/// One unit of best-effort warming for a track whose audio is already
/// cached. Each unit is a single bounded HTTP fetch so the worker can
/// re-check for higher-priority audio work between units.
#[derive(Debug, Clone)]
enum WarmUnit {
    Waveform { rating_key: String, audio_path: PathBuf },
    Art { thumb: String },
}

impl WarmUnit {
    /// Per-cycle dedupe key for the `warm_failed` set. Prefixed by kind so
    /// a track's distinct artefacts never collide on the same rating key.
    fn failed_key(&self) -> String {
        match self {
            WarmUnit::Waveform { rating_key, .. } => format!("wave:{rating_key}"),
            WarmUnit::Art { thumb } => format!("art:{thumb}"),
        }
    }

    /// Event payload announcing this unit's artefact is now on disk.
    fn warmed_payload(&self) -> MetadataWarmedPayload {
        match self {
            WarmUnit::Waveform { rating_key, .. } => MetadataWarmedPayload {
                kind: "waveform",
                rating_key: Some(rating_key.clone()),
                thumb: None,
            },
            WarmUnit::Art { thumb } => MetadataWarmedPayload {
                kind: "art",
                rating_key: None,
                thumb: Some(thumb.clone()),
            },
        }
    }
}

/// Find the next missing warm artefact across the lookahead window's
/// already-cached tracks, skipping anything that failed earlier this cycle.
/// Per track the waveform sidecar is filled before the hero art; returns
/// `None` when every in-window track is fully warmed.
fn next_warm_unit(
    app: &AppHandle,
    player: &AudioPlayer,
    warm_failed: &HashSet<String>,
) -> Option<WarmUnit> {
    let image_cache = &app.state::<crate::state::AppState>().image_cache;
    for target in player.lookahead_warm_targets(true) {
        let wave = WarmUnit::Waveform {
            rating_key: target.rating_key.clone(),
            audio_path: target.audio_path.clone(),
        };
        if !warm_failed.contains(&wave.failed_key())
            && !crate::commands::downloads::waveform_sidecar_path(&target.audio_path).is_file()
        {
            return Some(wave);
        }

        if let Some(thumb) = &target.thumb {
            let art = WarmUnit::Art {
                thumb: thumb.clone(),
            };
            if !warm_failed.contains(&art.failed_key()) {
                let cached = image_cache.lock().get(thumb, HERO_ART_SIZE).is_some();
                if !cached {
                    return Some(art);
                }
            }
        }
    }
    None
}

/// Execute one warm unit. Returns `true` once the artefact is present on
/// disk, `false` on any failure so the caller can suppress same-cycle
/// retries.
async fn run_warm_unit(app: &AppHandle, unit: &WarmUnit) -> bool {
    let state = app.state::<crate::state::AppState>();
    match unit {
        WarmUnit::Waveform {
            rating_key,
            audio_path,
        } => {
            crate::commands::downloads::warm_waveform_sidecar(&state.client, rating_key, audio_path)
                .await;
            crate::commands::downloads::waveform_sidecar_path(audio_path).is_file()
        }
        WarmUnit::Art { thumb } => {
            crate::commands::downloads::warm_art_size_unpinned(
                &state.image_cache,
                &state.client,
                &state.http_client,
                thumb,
                HERO_ART_SIZE,
            )
            .await
        }
    }
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

    // Download into a `.part` name and rename only on completion. The
    // startup rehydration scans this directory and can't tell a half-written
    // file from a complete one — an app kill mid-prefetch used to rehydrate
    // the stump as a complete cached track, which then played truncated (and
    // poisoned recovery, which treats a cached track's failure as a local
    // decode problem). The `.part` suffix fails rehydration's filename
    // parse, and an interrupted partial still resumes across restarts:
    // `download_http_to_file` Range-resumes whatever is on disk at the path.
    let part_path = cache_dir.join(format!("{filename}.part"));
    let size = download_http_to_file(client, url, &part_path, |_bytes, _total| {}).await?;
    tokio::fs::rename(&part_path, &file_path)
        .await
        .map_err(|e| format!("finalize rename: {e}"))?;

    let current_id = player.current_track_id();
    let evicted = player.with_cache(|cache| {
        cache.insert(track_id.to_string(), file_path.clone(), size);
        cache.evict_if_needed(current_id.as_deref())
    });
    for path in evicted {
        let wave = crate::commands::downloads::waveform_sidecar_path(&path);
        let lyrics = crate::commands::downloads::lyrics_sidecar_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&wave).await;
        let _ = tokio::fs::remove_file(&lyrics).await;
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

    // Transcoded jobs always carry "opus" in `job.codec`, but the
    // on-disk container is Ogg/Opus — match the prefetch worker's
    // extension and use ".ogg" so both worker paths produce the same
    // filename shape, and rehydration / lossless-codec checks line up.
    let ext = if ramus_core::playback::transcode::is_transcode_download_url(&job.url) {
        "ogg".to_string()
    } else {
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
            if progress_tick_due(last_emit.elapsed(), || {
                crate::events::webview_hidden(&app_for_cb)
            }) {
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
        let title = job.title.clone();
        let artist = job.artist_name.clone();
        let album = job.album_title.clone();
        let duration = job.duration;
        let file_path_warm = file_path.clone();
        tauri::async_runtime::spawn(async move {
            let state = app_warm.state::<crate::state::AppState>();
            if !state.player.has_persistent_download(&rk) {
                return;
            }
            crate::commands::downloads::warm_waveform_sidecar(&state.client, &rk, &file_path_warm)
                .await;
            if crate::commands::downloads::waveform_sidecar_path(&file_path_warm).is_file() {
                emit_metadata_warmed(
                    &app_warm,
                    MetadataWarmedPayload {
                        kind: "waveform",
                        rating_key: Some(rk.clone()),
                        thumb: None,
                    },
                );
            }
            // Permanent downloads are for offline use, so secure lyrics now too.
            crate::commands::downloads::warm_lyrics_sidecar(
                &state.client,
                &state.http_client,
                &rk,
                &file_path_warm,
                &title,
                &artist,
                &album,
                duration,
            )
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
                emit_metadata_warmed(
                    &app_warm,
                    MetadataWarmedPayload {
                        kind: "art",
                        rating_key: None,
                        thumb: Some(thumb),
                    },
                );
            }
        });
    }

    // If the downloaded track sits in the current playback queue, swap its
    // mpv playlist entry to the local file so the next time we hit that
    // track we read from disk.
    player.swap_playlist_entry_to_cached(&job.rating_key);

    emit_download_progress(app, progress_payload(job, "done", size, Some(size), None));
    emit_downloads_changed(app);

    log::info!("downloads: stored {} ({size} bytes)", job.rating_key);
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{progress_tick_due, DrainGate, PROGRESS_EMIT_INTERVAL};
    use std::time::Duration;

    #[test]
    fn drain_gate_holds_work_behind_a_live_stream() {
        assert!(DrainGate::default().due(|| true));
    }

    #[test]
    fn drain_gate_waits_for_a_stream_that_starts_mid_cycle() {
        // A restored queue: nothing streams when the cycle starts, then the
        // user presses Play and mpv opens the live stream between jobs.
        let gate = DrainGate::default();
        assert!(!gate.due(|| false));
        assert!(gate.due(|| true));
    }

    #[test]
    fn drain_gate_waits_once_per_cycle() {
        let mut gate = DrainGate::default();
        gate.drained();
        assert!(!gate.due(|| panic!("player read after the cycle already waited")));
    }

    #[test]
    fn progress_tick_goes_out_once_the_interval_has_passed() {
        assert!(progress_tick_due(PROGRESS_EMIT_INTERVAL, || false));
    }

    #[test]
    fn progress_tick_waits_for_the_interval() {
        let early = PROGRESS_EMIT_INTERVAL - Duration::from_millis(1);
        assert!(!progress_tick_due(early, || panic!("visibility read before the tick was due")));
    }

    #[test]
    fn progress_tick_skips_a_hidden_webview() {
        assert!(!progress_tick_due(PROGRESS_EMIT_INTERVAL * 4, || true));
    }
}
