//! Connection-aware prefetch worker with concurrent LAN downloads.
//!
//! ## What this module does
//!
//! Downloads upcoming tracks in the queue to the local audio cache so
//! gapless playback stays gapless even if the user skips around. A
//! secondary job is to generate per-track FFT spectrograms (`.spec`
//! files) for the focus-mode visualiser.
//!
//! ## Dual-mode design: LAN vs Remote
//!
//! The worker detects whether the current Plex connection is local
//! (LAN) or remote and adjusts its strategy:
//!
//! **LAN mode** — aggressive, matching the iOS port's approach:
//!   - No idle-wait: downloads start ~1-2s after track change
//!   - 3 concurrent downloads via `tokio::sync::Semaphore`
//!   - Targets are batch-snapshotted and dispatched in parallel
//!
//! **Remote mode** — conservative, preserving the original design:
//!   - Waits for mpv's `cache-speed` to hit 0 before starting
//!   - 10-20s safety gaps after track changes
//!   - Serial downloads (one at a time)
//!   - Plex servers are known to kill concurrent remote connections
//!
//! ## First-track FFT via mpv `stream-record`
//!
//! mpv writes the bytes it reads to a file via the `stream-record`
//! option (set per-loadfile in `player.rs::load_queue`). When the
//! worker's cycle starts, it runs symphonia on the stream-record file
//! to generate a `.spec` without opening an extra HTTP connection.
//!
//! ## Local-first playback
//!
//! As soon as a track lands in `DownloadCache` — from either a worker
//! download or a stream-record ingest — its mpv playlist entry gets
//! swapped to `file://<path>`. When mpv naturally advances to that
//! track, it reads from disk with zero network traffic.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ramus_core::playback::player::{is_allowed_extension, sanitize_filename, AudioPlayer};
use ramus_core::playback::spectrum::{read_spec_file, spec_file_path, SpectrumState};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::emit_spectrum_ready;
use crate::spectrum_analyzer;

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Per-track download time budget. Retries as many times as needed
/// within this window. Fast LAN finishes in 1-2 retries; throttled
/// connections (e.g. Windows Defender network inspection) get dozens of
/// small-chunk resumes. 90s is enough for a 40MB FLAC at ~500KB/s
/// effective throughput.
const DOWNLOAD_TIME_BUDGET: Duration = Duration::from_secs(90);

/// Hard cap on retries even within the time budget — safety net against
/// infinite loops if the server responds but sends 0-byte chunks.
const MAX_RETRIES: u32 = 60;

/// Initial backoff between resume attempts. Doubles on consecutive
/// retries that make no progress, resets when real progress is made.
const INITIAL_BACKOFF: Duration = Duration::from_millis(200);

/// Maximum backoff between resume attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// A retry must gain at least this many bytes to count as "progress"
/// for backoff-reset purposes.
const MIN_PROGRESS_BYTES: u64 = 4096;

// --- LAN mode ---

/// Max concurrent prefetch downloads on LAN. Serial (1) keeps total
/// connections at 2 (1 prefetch + 1 mpv). Plex servers actively reset
/// streams when they see 3+ concurrent downloads, causing resumes even
/// on fast LANs. Serial is barely slower in practice — a 40MB FLAC at
/// gigabit speed takes ~0.5s, so 6 tracks finish in ~3s vs ~2s with
/// concurrency. The time-budget retry strategy handles the rare resume
/// if Plex still complains.
const LAN_CONCURRENCY: usize = 1;

/// Delay before starting prefetch on LAN after a natural advance.
/// Just enough for mpv to issue its initial request.
const LAN_NATURAL_GAP: Duration = Duration::from_secs(1);

/// Delay after a user skip on LAN. Slightly longer so rapid skips
/// don't fire pointless downloads, but still fast.
const LAN_SKIP_GAP: Duration = Duration::from_secs(2);

// --- Remote mode ---

/// Delay after mpv network-idle detection on a natural advance.
const REMOTE_NATURAL_GAP: Duration = Duration::from_secs(10);

/// Delay after mpv network-idle detection on a user skip.
const REMOTE_SKIP_GAP: Duration = Duration::from_secs(20);

/// How long `cache-speed` must read 0 before we declare mpv idle.
/// Only used in remote mode.
const IDLE_SIGNAL_REQUIRED_DURATION: Duration = Duration::from_secs(3);

/// Fallback — if the idle signal never fires (e.g. file > demuxer-max-bytes),
/// start the prefetch loop anyway after this long. Remote mode only.
const IDLE_SIGNAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Poll interval while waiting for mpv to go idle. Remote mode only.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum PrefetchCmd {
    /// mpv's playlist-pos advanced naturally (gapless auto-advance or
    /// play_tracks kicking off a fresh queue).
    NaturalAdvance { generation: u64 },
    /// User-initiated skip (next/prev/jump). Aborts any in-flight
    /// download and restarts with a skip gap.
    Skipped { generation: u64 },
    /// Album switch / stop. Abort in-flight, wait for the next command
    /// before doing anything.
    #[allow(dead_code)]
    Cancel { generation: u64 },
}

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

/// Control surface for the background prefetch worker. Cloneable so it
/// can live in `AppState` and be called from any command handler or
/// event callback.
#[derive(Clone)]
pub struct PrefetchHandle {
    tx: mpsc::UnboundedSender<PrefetchCmd>,
    generation: Arc<AtomicU64>,
}

impl PrefetchHandle {
    /// Signal that mpv has naturally advanced to a new track.
    pub fn notify_natural_advance(&self) {
        let gen = self.generation.load(Ordering::SeqCst);
        let _ = self.tx.send(PrefetchCmd::NaturalAdvance { generation: gen });
    }

    /// Signal that the user skipped to a different track. Aborts
    /// in-flight work, then schedules a new cycle.
    pub fn notify_skip(&self) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(PrefetchCmd::Skipped { generation: gen });
    }

    /// Signal that the queue was replaced or playback was stopped.
    /// Aborts in-flight work; no new cycle is scheduled until the next
    /// natural advance.
    pub fn notify_cancel(&self) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.tx.send(PrefetchCmd::Cancel { generation: gen });
    }
}

/// Spawn the single long-lived prefetch worker task. Returns the handle
/// that callers use to pump commands in. Call this once at app startup.
pub fn spawn_worker(
    player: Arc<AudioPlayer>,
    http_client: reqwest::Client,
    app: AppHandle,
) -> PrefetchHandle {
    // Rehydrate the in-memory DownloadCache from any audio files already
    // on disk from a previous session.
    if let Ok(cfg_dir) = ramus_core::plex::token_store::config_dir() {
        rehydrate_cache_from_disk(&player, &cfg_dir.join("audio_cache"));
    }

    let (tx, rx) = mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(0));

    let worker_gen = generation.clone();
    tauri::async_runtime::spawn(async move {
        worker_loop(player, http_client, app, rx, worker_gen).await;
    });

    PrefetchHandle { tx, generation }
}

/// Scan the audio cache directory and register any files matching the
/// `<rating_key>_<len>.<ext>` naming convention into the in-memory
/// `DownloadCache`. Runs once at worker startup.
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

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

async fn worker_loop(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    mut rx: mpsc::UnboundedReceiver<PrefetchCmd>,
    shared_gen: Arc<AtomicU64>,
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
                // Natural advance: let in-flight finish naturally — its
                // loop will pick up the shifted window automatically.
                // Only spawn a fresh cycle if idle.
                let idle = cycle_task.as_ref().is_none_or(|h| h.is_finished());
                if idle {
                    log::debug!(
                        "prefetch: natural advance, starting cycle gen={generation}"
                    );
                    player.mark_network_active();
                    cycle_task = Some(spawn_cycle(
                        player.clone(),
                        http.clone(),
                        app.clone(),
                        shared_gen.clone(),
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
                player.mark_network_active();
                cycle_task = Some(spawn_cycle(
                    player.clone(),
                    http.clone(),
                    app.clone(),
                    shared_gen.clone(),
                    generation,
                    true,
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
    my_gen: u64,
    is_skip: bool,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_cycle(player, http, app, shared_gen, my_gen, is_skip).await;
    })
}

// ---------------------------------------------------------------------------
// Cycle — branches on LAN vs Remote
// ---------------------------------------------------------------------------

async fn run_cycle(
    player: Arc<AudioPlayer>,
    http: reqwest::Client,
    app: AppHandle,
    shared_gen: Arc<AtomicU64>,
    my_gen: u64,
    is_skip: bool,
) {
    let is_remote = player.is_remote();

    let gap = match (is_remote, is_skip) {
        (true, false) => REMOTE_NATURAL_GAP,
        (true, true) => REMOTE_SKIP_GAP,
        (false, false) => LAN_NATURAL_GAP,
        (false, true) => LAN_SKIP_GAP,
    };

    // Phase 1: idle wait (remote only — on LAN we have bandwidth to spare)
    if is_remote
        && !wait_for_mpv_network_idle(&player, &shared_gen, my_gen).await
    {
        return; // superseded
    }

    // Phase 2: safety gap
    tokio::time::sleep(gap).await;
    if shared_gen.load(Ordering::SeqCst) != my_gen {
        return;
    }

    // Phase 3: stream-record ingest (remote only — on LAN the prefetch
    // worker downloads the current track directly, so there's no
    // stream-record file to ingest).
    if is_remote {
        if let Some(current_id) = player.current_track_id() {
            try_ingest_stream_record(&player, &app, &current_id);
        }
    }

    // Phase 4: download
    let cache_dir = match ramus_core::plex::token_store::config_dir() {
        Ok(dir) => dir.join("audio_cache"),
        Err(_) => return,
    };
    if let Err(e) = tokio::fs::create_dir_all(&cache_dir).await {
        log::debug!("prefetch: cache dir create failed: {e}");
        return;
    }

    if is_remote {
        log::debug!("prefetch: remote mode — serial downloads");
        run_serial_downloads(&player, &http, &app, &shared_gen, my_gen, &cache_dir).await;
    } else {
        log::debug!("prefetch: LAN mode — {LAN_CONCURRENCY} concurrent downloads");
        run_concurrent_downloads(
            &player,
            &http,
            &app,
            &shared_gen,
            my_gen,
            &cache_dir,
            LAN_CONCURRENCY,
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Serial downloads (remote mode — preserves original behaviour)
// ---------------------------------------------------------------------------

async fn run_serial_downloads(
    player: &Arc<AudioPlayer>,
    http: &reqwest::Client,
    app: &AppHandle,
    shared_gen: &Arc<AtomicU64>,
    my_gen: u64,
    cache_dir: &std::path::Path,
) {
    let mut failed_in_cycle: HashSet<String> = HashSet::new();

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            log::debug!("prefetch: serial cycle superseded, exiting");
            return;
        }

        let Some((track_id, url)) = player.next_uncached_target_in_lookahead() else {
            log::debug!("prefetch: lookahead window exhausted (serial), idle");
            return;
        };

        if failed_in_cycle.contains(&track_id) {
            log::debug!(
                "prefetch: {track_id} already failed in this cycle, ending"
            );
            return;
        }

        match download_with_resume(player, http, cache_dir, &track_id, &url).await {
            Ok(()) => {
                player.swap_playlist_entry_to_cached(&track_id);
                spawn_analyse_task(player, track_id, app.clone());
            }
            Err(e) => {
                log::debug!("prefetch: serial download failed for {track_id}: {e}");
                failed_in_cycle.insert(track_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Concurrent downloads (LAN mode)
// ---------------------------------------------------------------------------

async fn run_concurrent_downloads(
    player: &Arc<AudioPlayer>,
    http: &reqwest::Client,
    app: &AppHandle,
    shared_gen: &Arc<AtomicU64>,
    my_gen: u64,
    cache_dir: &std::path::Path,
    max_concurrent: usize,
) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let mut join_set = tokio::task::JoinSet::new();
    let failed_in_cycle: Arc<std::sync::Mutex<HashSet<String>>> =
        Arc::new(std::sync::Mutex::new(HashSet::new()));

    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            join_set.shutdown().await;
            return;
        }

        let targets = player.all_uncached_targets_in_lookahead(true);

        // Filter out targets that already failed this cycle.
        let targets: Vec<_> = {
            let failed = failed_in_cycle.lock().unwrap();
            targets
                .into_iter()
                .filter(|(id, _)| !failed.contains(id))
                .collect()
        };

        if targets.is_empty() {
            // Wait for any in-flight downloads to finish, then re-check
            // in case the window shifted (user advanced).
            if join_set.is_empty() {
                log::debug!("prefetch: lookahead exhausted (concurrent), idle");
                return;
            }
            let _ = join_set.join_next().await;
            continue;
        }

        for (i, (track_id, url)) in targets.into_iter().enumerate() {
            if shared_gen.load(Ordering::SeqCst) != my_gen {
                join_set.shutdown().await;
                return;
            }

            // Stagger launches so concurrent requests don't hit the
            // server in the same instant, which triggers rate limiting.
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }

            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => return, // semaphore closed
            };

            let player = player.clone();
            let http = http.clone();
            let app = app.clone();
            let gen = shared_gen.clone();
            let cache_dir = cache_dir.to_path_buf();
            let failed = failed_in_cycle.clone();

            join_set.spawn(async move {
                let _permit = permit; // held until task completes

                if gen.load(Ordering::SeqCst) != my_gen {
                    return;
                }

                match download_with_resume(&player, &http, &cache_dir, &track_id, &url).await {
                    Ok(()) => {
                        player.swap_playlist_entry_to_cached(&track_id);
                        spawn_analyse_task(&player, track_id, app);
                    }
                    Err(e) => {
                        log::debug!(
                            "prefetch: concurrent download failed for {track_id}: {e}"
                        );
                        failed.lock().unwrap().insert(track_id);
                    }
                }
            });
        }

        // Wait for all dispatched downloads to complete before
        // re-checking the window. Prevents re-dispatching targets
        // that are still in-flight.
        while join_set.join_next().await.is_some() {}
    }
}

// ---------------------------------------------------------------------------
// Idle detection (remote mode only)
// ---------------------------------------------------------------------------

/// Poll `player.mpv_network_idle_for()` until it returns true, or until
/// `IDLE_SIGNAL_TIMEOUT` elapses. Returns false if the current generation
/// has moved on (cycle superseded).
async fn wait_for_mpv_network_idle(
    player: &AudioPlayer,
    shared_gen: &Arc<AtomicU64>,
    my_gen: u64,
) -> bool {
    let start = Instant::now();
    loop {
        if shared_gen.load(Ordering::SeqCst) != my_gen {
            return false;
        }
        if player.mpv_network_idle_for(IDLE_SIGNAL_REQUIRED_DURATION) {
            log::debug!("prefetch: mpv idle detected");
            return true;
        }
        if start.elapsed() >= IDLE_SIGNAL_TIMEOUT {
            log::debug!("prefetch: idle signal timeout, proceeding anyway");
            return true;
        }
        tokio::time::sleep(IDLE_POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// Stream-record ingest
// ---------------------------------------------------------------------------

/// Attempt to decode the stream-record file mpv has been writing for a
/// track, persist a `.spec`, insert the audio into `DownloadCache`, and
/// swap mpv's playlist entry to `file://`. Fire-and-forget.
pub fn try_ingest_stream_record(player: &Arc<AudioPlayer>, app: &AppHandle, track_id: &str) {
    let Some(path) = player.stream_record_path_for(track_id) else {
        return;
    };
    if !path.exists() {
        return;
    }
    let spectrum_disabled = app
        .state::<crate::state::AppState>()
        .settings
        .read()
        .disable_spectrum;
    // Already ingested — skip re-analysis.
    if player.with_cache(|c| c.get(track_id).is_some()) {
        // But still make sure the `.spec` is on disk for the viz.
        if !spectrum_disabled && read_spec_file(&path).is_none() {
            spawn_ingest_analysis(player.clone(), app.clone(), track_id.to_string(), path, false);
        }
        return;
    }
    spawn_ingest_analysis(
        player.clone(),
        app.clone(),
        track_id.to_string(),
        path,
        spectrum_disabled,
    );
}

fn spawn_ingest_analysis(
    player: Arc<AudioPlayer>,
    app: AppHandle,
    track_id: String,
    audio_path: PathBuf,
    skip_spectrum: bool,
) {
    tokio::task::spawn_blocking(move || {
        // Always try to decode the file so we can ingest it into the
        // download cache (local-first playback). Spectrum analysis is
        // a bonus that rides on the same decode pass.
        match spectrum_analyzer::analyse_file(&audio_path) {
            Ok(frames) => {
                let size = std::fs::metadata(&audio_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let current = player.current_track_id();
                let evicted = player.with_cache(|cache| {
                    cache.insert(track_id.clone(), audio_path.clone(), size);
                    cache.evict_if_needed(current.as_deref())
                });
                if !skip_spectrum {
                    let state = SpectrumState::Ready(frames);
                    let _ =
                        ramus_core::playback::spectrum::write_spec_file(&audio_path, &state);
                    emit_spectrum_ready(&app, track_id.clone());
                }
                player.swap_playlist_entry_to_cached(&track_id);
                for p in evicted {
                    let spec = spec_file_path(&p);
                    let _ = std::fs::remove_file(&p);
                    let _ = std::fs::remove_file(&spec);
                }
            }
            Err(err) => {
                log::debug!(
                    "prefetch: stream-record probe failed for {track_id}: {err}, deleting {audio_path:?}"
                );
                let _ = std::fs::remove_file(&audio_path);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Spectrum analysis (for files downloaded via the worker)
// ---------------------------------------------------------------------------

fn spawn_analyse_task(player: &AudioPlayer, track_id: String, app: AppHandle) {
    if app.state::<crate::state::AppState>().settings.read().disable_spectrum {
        return;
    }
    let Some(audio_path) = player.with_cache(|c| c.get(&track_id).map(|p| p.to_path_buf()))
    else {
        return;
    };
    if read_spec_file(&audio_path).is_some() {
        emit_spectrum_ready(&app, track_id);
        return;
    }
    tokio::task::spawn_blocking(move || {
        spectrum_analyzer::analyse_and_persist(&audio_path);
        emit_spectrum_ready(&app, track_id);
    });
}

// ---------------------------------------------------------------------------
// Downloader (shared by both LAN and Remote paths)
// ---------------------------------------------------------------------------

/// Download a file using resumable Range requests. Cancellation is
/// external: callers wrap this in a `tokio::spawn` and abort if needed.
/// Partial files are kept on abort so a follow-up cycle can resume.
async fn download_with_resume(
    player: &AudioPlayer,
    client: &reqwest::Client,
    cache_dir: &std::path::Path,
    track_id: &str,
    url: &str,
) -> Result<(), String> {
    if player.with_cache(|c| c.get(track_id).is_some()) {
        return Ok(());
    }

    let ext = url::Url::parse(url)
        .ok()
        .and_then(|u| u.path().rsplit('.').next().map(|e| e.to_lowercase()))
        .filter(|e| is_allowed_extension(e))
        .unwrap_or_else(|| "bin".to_string());

    let filename = format!(
        "{}_{}.{}",
        sanitize_filename(track_id),
        track_id.len(),
        ext
    );
    let file_path = cache_dir.join(&filename);

    // Resume from any existing partial file.
    let mut written: u64 = tokio::fs::metadata(&file_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&file_path)
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
                let now = Instant::now();
                if retries >= MAX_RETRIES || now >= deadline {
                    return Err(format!("request error after {retries} retries: {e}"));
                }
                log::debug!(
                    "prefetch {track_id}: request error (attempt {retries}): {e}"
                );
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
            log::debug!("prefetch {track_id}: {status}, content-length={cl}");
            expected_size = cl.parse().ok().map(|cl: u64| cl + written);
        }

        // 416 Range Not Satisfiable: stale/complete partial on disk.
        if status.as_u16() == 416 {
            drop(file);
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(format!(
                "HTTP 416 — stale partial at {} bytes removed, will retry next cycle",
                written
            ));
        }

        if !status.is_success() && status.as_u16() != 206 {
            return Err(format!("HTTP {status}"));
        }

        // Server ignored our Range header — start over from scratch.
        if written > 0 && status.as_u16() == 200 {
            written = 0;
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|e| e.to_string())?;
            file.set_len(0).await.map_err(|e| e.to_string())?;
        }

        let mut chunk_error = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| format!("write error: {e}"))?;
                    written += chunk.len() as u64;
                }
                Ok(None) => break,
                Err(e) => {
                    log::debug!(
                        "prefetch {track_id}: chunk error at {written} bytes: {e}"
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
        let bytes_this_attempt = written - written_before_attempt;

        if retries >= MAX_RETRIES || now >= deadline {
            return Err(format!(
                "gave up after {retries} retries ({:.1}s): got {written} of {} bytes",
                (now - download_start).as_secs_f64(),
                expected_size
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
        }

        // Adaptive backoff: reset when making real progress, escalate
        // when stalled. Keeps fast connections fast while backing off
        // when the server is actively throttling.
        if bytes_this_attempt >= MIN_PROGRESS_BYTES {
            current_backoff = INITIAL_BACKOFF;
        } else {
            current_backoff = (current_backoff * 2).min(MAX_BACKOFF);
        }

        let remaining = deadline.saturating_duration_since(now);
        log::debug!(
            "prefetch {track_id}: resuming at {written}/{} \
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
    let size = written;

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

    log::debug!("prefetch: cached {track_id} ({size} bytes, {retries} resumes)");
    Ok(())
}
