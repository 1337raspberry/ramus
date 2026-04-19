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

/// Delay before starting prefetch after a natural advance; enough for mpv
/// to issue its initial request.
const NATURAL_GAP: Duration = Duration::from_secs(1);

/// Delay after a user skip. Slightly longer so rapid skips don't fire
/// pointless downloads.
const SKIP_GAP: Duration = Duration::from_secs(2);

/// Emit progress events at most this often per in-flight download. Bytes
/// ticks at chunk rate (>50Hz on LAN); throttling keeps the event bus quiet.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(250);

// --- Public types ---

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
        let _ = self.tx.send(PrefetchCmd::NaturalAdvance { generation: gen });
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
                let already_queued = s
                    .user_queue
                    .iter()
                    .any(|q| q.rating_key == job.rating_key);
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

    /// Cancel every queued and in-flight user download.
    pub fn cancel_all_user_downloads(&self) {
        let had_in_flight = {
            let mut s = self.shared.lock();
            s.user_queue.clear();
            s.in_progress.is_some()
        };
        if had_in_flight {
            let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = self
                .tx
                .send(PrefetchCmd::CancelUserDownload { generation: gen });
        }
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
        if path.is_file() {
            entries.insert(rating_key, path);
        } else {
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

async fn run_cycle(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    shared_gen: Arc<AtomicU64>,
    shared: Arc<Mutex<Shared>>,
    my_gen: u64,
    is_skip: bool,
) {
    let gap = if is_skip { SKIP_GAP } else { NATURAL_GAP };

    // Safety gap so mpv issues its initial request first.
    tokio::time::sleep(gap).await;
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

    // Include the currently-playing track when spectrum analysis is enabled,
    // since the download gives the FFT a clean file. When disabled, skip it
    // (swap-to-local is a no-op for the active mpv index anyway).
    let spectrum_disabled = app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum;
    let include_current = !spectrum_disabled;

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
        if include_current { " (incl. current)" } else { "" },
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
            match run_user_download(player, http, app, shared, shared_gen, my_gen, downloads_dir, &job).await {
                Ok(()) => {}
                Err(e) => {
                    log::warn!("downloads: user download failed for {}: {e}", job.rating_key);
                    emit_download_progress(
                        app,
                        DownloadProgressPayload {
                            rating_key: job.rating_key.clone(),
                            album_rating_key: job.album_rating_key.clone(),
                            title: job.title.clone(),
                            artist_name: job.artist_name.clone(),
                            album_title: job.album_title.clone(),
                            thumb: job.thumb.clone(),
                            phase: "failed",
                            bytes_written: 0,
                            total_bytes: job.expected_size_bytes,
                            error: Some(e),
                        },
                    );
                    user_failed.insert(job.rating_key);
                }
            }
            // Loop back to pick up the next user job (or fall through to prefetch).
            continue;
        }

        // No user work — fall back to prefetch.
        let Some((track_id, url)) = player.next_uncached_target_in_lookahead(include_current) else {
            log::debug!("prefetch: lookahead window exhausted, idle");
            return;
        };

        if prefetch_failed.contains(&track_id) {
            log::debug!("prefetch: {track_id} already failed this cycle, ending");
            return;
        }

        match run_prefetch_download(player, http, app, prefetch_dir, &track_id, &url).await {
            Ok(()) => {
                player.swap_playlist_entry_to_cached(&track_id);
                spawn_analyse_task_from_cache(player, track_id, app.clone());
            }
            Err(e) => {
                log::debug!("prefetch: serial download failed for {track_id}: {e}");
                prefetch_failed.insert(track_id);
            }
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
    let filename = format!(
        "{}_{}.{}",
        sanitize_filename(track_id),
        track_id.len(),
        ext
    );
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
            DownloadProgressPayload {
                rating_key: job.rating_key.clone(),
                album_rating_key: job.album_rating_key.clone(),
                title: job.title.clone(),
                artist_name: job.artist_name.clone(),
                album_title: job.album_title.clone(),
                thumb: job.thumb.clone(),
                phase: "done",
                bytes_written: job.expected_size_bytes.unwrap_or(0),
                total_bytes: job.expected_size_bytes,
                error: None,
            },
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
    let filename = format!("{}.{}", sanitize_filename(&job.rating_key), ext);
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
        DownloadProgressPayload {
            rating_key: job.rating_key.clone(),
            album_rating_key: job.album_rating_key.clone(),
            title: job.title.clone(),
            artist_name: job.artist_name.clone(),
            album_title: job.album_title.clone(),
            thumb: job.thumb.clone(),
            phase: "downloading",
            bytes_written: 0,
            total_bytes: job.expected_size_bytes,
            error: None,
        },
    );

    let rk = job.rating_key.clone();
    let alb = job.album_rating_key.clone();
    let title = job.title.clone();
    let artist = job.artist_name.clone();
    let album_title = job.album_title.clone();
    let thumb = job.thumb.clone();
    let app_for_cb = app.clone();
    let shared_for_cb = shared.clone();
    let expected = job.expected_size_bytes;
    let mut last_emit = Instant::now();

    let download_result = download_http_to_file(client, &job.url, &file_path, move |bytes, total| {
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
                DownloadProgressPayload {
                    rating_key: rk.clone(),
                    album_rating_key: alb.clone(),
                    title: title.clone(),
                    artist_name: artist.clone(),
                    album_title: album_title.clone(),
                    thumb: thumb.clone(),
                    phase: "downloading",
                    bytes_written: bytes,
                    total_bytes: total,
                    error: None,
                },
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
    {
        let state = app.state::<crate::state::AppState>();
        let cache_guard = state.cache.lock();
        if let Some(cache) = cache_guard.as_ref() {
            if let Err(e) = cache.insert_download(&row) {
                log::warn!("downloads: insert_download failed: {e}");
            }
        }
    }

    // Register for local-first playback and skip iOS backup.
    player.register_persistent_download(job.rating_key.clone(), file_path.clone());
    ios_backup::exclude_from_backup(&file_path);

    // Warm the ancillary caches so offline playback has everything the UI
    // needs: waveform sidecar for the seek bar, and album art pre-fetched
    // at every display size. Fire-and-forget — if the network drops
    // between audio download and these best-effort fetches, we just degrade
    // gracefully at render time.
    {
        let app_warm = app.clone();
        let rk = job.rating_key.clone();
        let thumb = job.thumb.clone();
        let file_path_warm = file_path.clone();
        tauri::async_runtime::spawn(async move {
            let state = app_warm.state::<crate::state::AppState>();
            crate::commands::downloads::warm_waveform_sidecar(
                &state.client,
                &rk,
                &file_path_warm,
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

    emit_download_progress(
        app,
        DownloadProgressPayload {
            rating_key: job.rating_key.clone(),
            album_rating_key: job.album_rating_key.clone(),
            title: job.title.clone(),
            artist_name: job.artist_name.clone(),
            album_title: job.album_title.clone(),
            thumb: job.thumb.clone(),
            phase: "done",
            bytes_written: size,
            total_bytes: Some(size),
            error: None,
        },
    );
    emit_downloads_changed(app);

    log::info!("downloads: stored {} ({size} bytes)", job.rating_key);
    Ok(())
}

// --- Spectrum analysis ---

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
    if app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum
    {
        return;
    }
    if read_spec_file(&audio_path).is_some() {
        emit_spectrum_ready(&app, track_id);
        return;
    }
    tokio::task::spawn_blocking(move || {
        spectrum_analyzer::analyse_and_persist(&audio_path);
        emit_spectrum_ready(&app, track_id);
    });
}

// --- Shared HTTP download core ---

fn extension_from_url(url: &str) -> String {
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
async fn download_http_to_file(
    client: &reqwest::Client,
    url: &str,
    file_path: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<u64, String> {
    let mut written: u64 = tokio::fs::metadata(file_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(file_path)
        .await
        .map_err(|e| format!("create file: {e}"))?;
    if written > 0 {
        file.seek(std::io::SeekFrom::Start(written))
            .await
            .map_err(|e| format!("seek partial: {e}"))?;
    }

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
                if Instant::now() >= deadline {
                    return Err(format!("request error after {retries} retries: {e}"));
                }
                log::debug!("download: request error (attempt {retries}): {e}");
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
                    on_progress(written, expected_size);
                }
                Ok(None) => break,
                Err(e) => {
                    log::debug!("download: chunk error at {written} bytes: {e}");
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
        let bytes_this_attempt = written - written_before_attempt;

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
    Ok(written)
}
