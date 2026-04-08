use std::path::PathBuf;
use std::sync::Arc;

use ramus_core::playback::player::{is_allowed_extension, sanitize_filename, AudioPlayer};

/// Spawn background prefetch tasks for upcoming tracks in the queue.
///
/// Identifies tracks that need downloading via `player.prefetch_targets()`,
/// then spawns async download tasks for each. Downloads are saved to the
/// audio cache directory and registered in the player's LRU cache.
pub fn trigger(player: Arc<AudioPlayer>, http_client: reqwest::Client) {
    let targets = player.prefetch_targets();
    if targets.is_empty() {
        return;
    }

    let cache_dir = match ramus_core::plex::token_store::config_dir() {
        Ok(dir) => dir.join("audio_cache"),
        Err(_) => return,
    };

    tauri::async_runtime::spawn(async move {
        // Brief delay to let the server finish setting up the current stream
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        for (track_id, url) in targets {
            // Re-check cache in case another trigger already downloaded this
            let cached = player.with_cache(|c| c.get(&track_id).is_some());
            if cached {
                continue;
            }
            if let Err(e) =
                download_and_cache(&player, &http_client, &cache_dir, &track_id, &url).await
            {
                log::debug!("prefetch failed for {track_id}: {e}");
            }
        }
    });
}

async fn download_and_cache(
    player: &AudioPlayer,
    client: &reqwest::Client,
    cache_dir: &PathBuf,
    track_id: &str,
    url: &str,
) -> Result<(), String> {
    // Double-check it wasn't cached while we were waiting to start
    let already_cached = player.with_cache(|c| c.get(track_id).is_some());
    if already_cached {
        return Ok(());
    }

    // Ensure cache directory exists
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|e| e.to_string())?;

    // Determine file extension from URL path
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

    // Download
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let size = bytes.len() as u64;

    // Write to disk
    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| e.to_string())?;

    // Register in cache and evict old entries
    let current_id = player.current_track_id();
    let evicted = player.with_cache(|cache| {
        cache.insert(track_id.to_string(), file_path, size);
        cache.evict_if_needed(current_id.as_deref())
    });

    // Clean up evicted files from disk
    for path in evicted {
        let _ = tokio::fs::remove_file(&path).await;
    }

    log::debug!("prefetch: cached {track_id} ({} bytes)", size);
    Ok(())
}
