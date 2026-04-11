use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ramus_core::playback::player::{is_allowed_extension, sanitize_filename, AudioPlayer};
use ramus_core::playback::spectrum::{read_spec_file, spec_file_path};
use tauri::AppHandle;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::events::emit_spectrum_ready;
use crate::spectrum_analyzer;

/// Monotonic counter — each `trigger()` call bumps this. Downloads check it
/// before starting each track; if the generation has moved on, the old batch
/// stops so we don't open competing connections that Plex will kill.
pub(crate) static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Max resume attempts per track before giving up.
const MAX_RETRIES: u32 = 60;

/// Spawn background prefetch + spectrum-analysis tasks.
///
/// There are two concurrent paths here:
///
/// 1. **Fast path — the current track.** Skips the buffer/min-wait dance
///    entirely and fires the download + analyser as soon as the track
///    starts. This is what gets the focus-mode visualiser up within a
///    few seconds on the very first track of a session (or any track
///    the user skips to). It's safe to do this without waiting because
///    direct-play tracks are plain static file serves — Plex's
///    session/concurrency limits only bit back in the transcode era.
///
/// 2. **Slow path — upcoming tracks.** Keeps the original buffer-wait
///    loop (`is_fully_buffered` + `min_wait`). These aren't time-critical
///    (they just need to be cached by the time the user reaches them in
///    the queue), and the wait is a cheap safety margin against mpv's
///    HTTP GET colliding with ours on slower links.
///
/// Both paths share a single `GENERATION` counter so skips and queue
/// reloads cleanly abort in-flight work. They also share
/// [`spawn_analyse_task`] for the "kick off symphonia + FFT +
/// emit_spectrum_ready" bit, which is why both call sites look so thin.
pub fn trigger(player: Arc<AudioPlayer>, http_client: reqwest::Client, app: AppHandle) {
    let cache_dir = match ramus_core::plex::token_store::config_dir() {
        Ok(dir) => dir.join("audio_cache"),
        Err(_) => return,
    };

    // Bump generation so any in-flight prefetch batch from a previous trigger
    // will notice and stop before starting its next download.
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    // --- Fast path: current track ---
    //
    // Only spawned if the current track has a direct-play target and
    // isn't already in the cache. Transcoded tracks return None here and
    // get handled by `get_spectrum`'s `would_transcode` check instead —
    // they never hit the analyser.
    if let Some((current_id, current_url)) = player.current_track_download_target() {
        let fp_player = player.clone();
        let fp_http = http_client.clone();
        let fp_app = app.clone();
        let fp_cache_dir = cache_dir.clone();
        tauri::async_runtime::spawn(async move {
            if GENERATION.load(Ordering::SeqCst) != gen {
                return;
            }
            match download_with_resume(
                &fp_player,
                &fp_http,
                &fp_cache_dir,
                &current_id,
                &current_url,
                gen,
            )
            .await
            {
                Ok(()) => {
                    if GENERATION.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    spawn_analyse_task(&fp_player, current_id, fp_app);
                }
                Err(e) => {
                    if GENERATION.load(Ordering::SeqCst) == gen {
                        log::debug!("current-track download failed: {e}");
                    }
                }
            }
        });
    }

    // --- Slow path: upcoming tracks ---
    //
    // Unchanged from the pre-Option-A behaviour — wait for mpv's buffer
    // to settle, then walk the upcoming queue downloading + analysing
    // each track in turn.
    let targets = player.prefetch_targets();
    if targets.is_empty() {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let start = tokio::time::Instant::now();
        let deadline = start + std::time::Duration::from_secs(120);
        let min_wait = std::time::Duration::from_secs(15);

        // Phase 1: wait for mpv's buffer to fill (playback is stable)
        loop {
            if GENERATION.load(Ordering::SeqCst) != gen {
                return;
            }
            if player.is_fully_buffered() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                log::debug!("prefetch: timed out waiting for buffer, starting anyway");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Phase 2: ensure minimum wait since trigger so mpv finishes
        // downloading the full file, not just filling its cache buffer
        let elapsed = start.elapsed();
        if elapsed < min_wait {
            let remaining = min_wait - elapsed;
            tokio::time::sleep(remaining).await;
        }

        if GENERATION.load(Ordering::SeqCst) != gen {
            return;
        }

        for (track_id, url) in targets {
            if GENERATION.load(Ordering::SeqCst) != gen {
                log::debug!("prefetch: batch superseded, stopping");
                return;
            }

            let cached = player.with_cache(|c| c.get(&track_id).is_some());
            if !cached {
                if let Err(e) = download_with_resume(
                    &player,
                    &http_client,
                    &cache_dir,
                    &track_id,
                    &url,
                    gen,
                )
                .await
                {
                    if GENERATION.load(Ordering::SeqCst) != gen {
                        log::debug!("prefetch: batch superseded, stopping");
                        return;
                    }
                    log::debug!("prefetch failed for {track_id}: {e}");
                    continue;
                }
            }

            spawn_analyse_task(&player, track_id, app.clone());
        }
    });
}

/// Fire-and-forget analyser task. Expects the audio file to already be
/// in the player's download cache (by rating_key). Runs symphonia +
/// FFT on `spawn_blocking` so the ~1-2 s CPU spike doesn't park a
/// tokio worker, then emits `spectrum-ready` so the focus visualiser
/// can pull the result via `get_spectrum`.
///
/// If a valid `.spec` file already sits next to the cached audio, we
/// skip the analyse pass entirely and just re-emit `spectrum-ready` so
/// the frontend hydrates from disk. Without this short-circuit every
/// `trigger()` call (i.e. every track change) would burn 1-2 s of CPU
/// re-analysing already-analysed upcoming tracks.
fn spawn_analyse_task(player: &AudioPlayer, track_id: String, app: AppHandle) {
    let Some(audio_path) = player.with_cache(|c| c.get(&track_id).map(|p| p.to_path_buf())) else {
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

/// Download a file using resumable Range requests. Plex's remote server
/// may drop connections after ~700KB — we resume from where we left off
/// until the full file is on disk.
async fn download_with_resume(
    player: &AudioPlayer,
    client: &reqwest::Client,
    cache_dir: &PathBuf,
    track_id: &str,
    url: &str,
    gen: u64,
) -> Result<(), String> {
    if player.with_cache(|c| c.get(track_id).is_some()) {
        return Ok(());
    }

    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|e| e.to_string())?;

    let ext = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path()
                .rsplit('.')
                .next()
                .map(|e| e.to_lowercase())
        })
        .filter(|e| is_allowed_extension(e))
        .unwrap_or_else(|| "bin".to_string());

    let filename = format!("{}_{}.{}", sanitize_filename(track_id), track_id.len(), ext);
    let file_path = cache_dir.join(&filename);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&file_path)
        .await
        .map_err(|e| format!("create file: {e}"))?;

    let mut written: u64 = 0;
    let mut expected_size: Option<u64> = None;
    let mut retries: u32 = 0;

    loop {
        if GENERATION.load(Ordering::SeqCst) != gen {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err("superseded".into());
        }

        let mut request = client.get(url);
        if written > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={written}-"));
        }

        let mut response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                retries += 1;
                if retries >= MAX_RETRIES {
                    let _ = tokio::fs::remove_file(&file_path).await;
                    return Err(format!("request error after {retries} retries: {e}"));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
        };

        let status = response.status();

        if written == 0 {
            let cl = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(none)")
                .to_string();
            log::debug!("prefetch {track_id}: {status}, content-length={cl}");
            expected_size = cl.parse().ok();
        }

        if !status.is_success() && status.as_u16() != 206 {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(format!("HTTP {status}"));
        }

        // Server ignored our Range header — start over
        if written > 0 && status.as_u16() == 200 {
            written = 0;
            file.seek(std::io::SeekFrom::Start(0)).await.map_err(|e| e.to_string())?;
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
                Err(_) => {
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
        if retries >= MAX_RETRIES {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(format!(
                "gave up after {retries} retries: got {written} of {} bytes",
                expected_size.map(|s| s.to_string()).unwrap_or_else(|| "unknown".into())
            ));
        }

        log::debug!(
            "prefetch {track_id}: resuming at {written}/{} (attempt {retries})",
            expected_size.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
        );

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    let size = written;

    let current_id = player.current_track_id();
    let evicted = player.with_cache(|cache| {
        cache.insert(track_id.to_string(), file_path, size);
        cache.evict_if_needed(current_id.as_deref())
    });

    for path in evicted {
        // Drop the sibling .spec alongside the audio file. The .spec
        // isn't tracked in DownloadCache so without this it would
        // accumulate on disk forever — orphaned spectrograms for
        // tracks whose audio is long gone.
        let spec = spec_file_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&spec).await;
    }

    log::debug!("prefetch: cached {track_id} ({size} bytes, {retries} resumes)");
    Ok(())
}
