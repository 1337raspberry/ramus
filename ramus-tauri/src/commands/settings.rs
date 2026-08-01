use serde::Serialize;
use tauri::{AppHandle, State};

use ramus_core::genre::mapper::GenreMapper;
use ramus_core::genre::markup::{build_description_segments, normalize_artist, DescriptionSegment};
use ramus_core::genre::parser::CustomGenreParser;
use ramus_core::models::{Bookmark, Settings};
use ramus_core::playback::spectrum::spec_file_path;

use crate::events::{emit_connection_status, ConnectionStatusPayload};
use crate::state::AppState;

use super::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCacheStats {
    pub entry_count: usize,
    pub total_size_bytes: u64,
    /// Subset of `entry_count` that is pinned for offline downloads —
    /// these survive `flush_image_cache`.
    pub pinned_count: usize,
    /// Subset of `total_size_bytes` that is pinned for offline downloads.
    pub pinned_size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCacheStats {
    pub entry_count: usize,
    pub total_size_bytes: u64,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.read().clone())
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<()> {
    Bookmark::validate_batch(&settings.bookmarks)?;

    let prev_genre_source = state.settings.read().genre_source;
    let prev_offline_mode = state.settings.read().offline_mode;

    let config = settings.to_playback_config();
    state.player.update_config(config);

    state
        .player
        .apply_equalizer(settings.eq_enabled, &settings.eq_bands);

    state
        .connection_monitor
        .set_allow_http(!settings.refuse_http);

    state
        .image_cache
        .lock()
        .set_limit(settings.image_cache_limit_bytes as u64);

    // Re-emit connection-status if the user toggled Work Offline — same
    // online state, but `effective_offline` flips with the manual flag.
    if settings.offline_mode != prev_offline_mode {
        let online = state
            .server_reachable
            .load(std::sync::atomic::Ordering::Acquire);
        emit_connection_status(
            &app,
            ConnectionStatusPayload {
                online,
                offline_mode_manual: settings.offline_mode,
                effective_offline: settings.offline_mode || !online,
            },
        );
    }

    // Reload genre mapper if source changed.
    if settings.genre_source != prev_genre_source {
        match settings.genre_source {
            ramus_core::models::GenreSource::Custom => {
                if let Some(data) = ramus_core::settings::load_custom_genres() {
                    if let Ok(mapper) = GenreMapper::from_json_bytes(&data) {
                        *state.genre_mapper.write() = Some(mapper);
                    }
                }
            }
            ramus_core::models::GenreSource::Open => {
                let open_json = include_bytes!("../../data/open.json");
                if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
                    *state.genre_mapper.write() = Some(mapper);
                } else {
                    *state.genre_mapper.write() = None;
                }
            }
        }
    }

    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    Ok(())
}

#[tauri::command]
pub async fn has_custom_genres() -> CmdResult<bool> {
    Ok(ramus_core::settings::load_custom_genres().is_some())
}

/// Upper bound on an imported genre JSON payload. Richer trees carry long-form
/// descriptions for thousands of genres, so this is far larger than the plain
/// text importer's cap — but still bounded against pathological input.
const MAX_GENRE_JSON_BYTES: usize = 64 * 1024 * 1024;

/// Import a pre-built genre tree from JSON (carrying optional per-genre
/// metadata) and make it the active custom tree. Unlike `import_custom_genres`,
/// which converts an indented plain-text outline, this consumes the richer JSON
/// shape directly and preserves its descriptions and reference names. Returns
/// the total genre count across all depths, for a confirmation message.
#[tauri::command]
pub async fn import_custom_genres_json(
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<usize> {
    if text.len() > MAX_GENRE_JSON_BYTES {
        return Err("genre file is too large".to_string());
    }
    let data = text.into_bytes();
    // Building the mapper validates the JSON shape; reject before persisting.
    let mapper = GenreMapper::from_json_bytes(&data).map_err(|e| e.to_string())?;
    let count = mapper.node_count();
    // A valid-but-empty tree (e.g. a truncated export reduced to the bare
    // envelope) would silently gut genre browsing until manually removed —
    // reject it like the plain-text importer does.
    if count == 0 {
        return Err("no genres found in file".to_string());
    }
    ramus_core::settings::save_custom_genres(&data).map_err(|e| e.to_string())?;
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Custom;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    *state.genre_mapper.write() = Some(mapper);
    Ok(count)
}

/// Display metadata for a single genre, with its description pre-segmented into
/// text + genre links + artist links (the latter resolved against the library's
/// artists). `None` when the active tree carries no metadata for the name.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreMetadataResponse {
    pub canonical_name: String,
    /// Whether this genre has albums in the library — drives whether the title
    /// is a navigable link (and is underlined) in the UI.
    pub in_library: bool,
    pub short_summary: Option<String>,
    pub cosmetic_aka: Vec<String>,
    /// Ordered, non-overlapping description segments. Empty when the genre has
    /// no long-form description.
    pub description_segments: Vec<DescriptionSegment>,
}

#[tauri::command]
pub async fn get_genre_metadata(
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<Option<GenreMetadataResponse>> {
    // Library membership flags each genre/artist reference. Both come from the
    // cache; if it isn't ready yet, fall back to empty sets (everything reads as
    // not-in-library — genre links still drill, they just aren't underlined).
    let mut artist_names: Vec<String> = super::with_cache(&state, |db| {
        Ok(db.all_artists()?.into_iter().map(|a| a.1).collect())
    })
    .unwrap_or_default();
    let mut genre_album_sets =
        super::with_cache(&state, |db| db.genre_album_sets()).unwrap_or_default();

    // Offline mode filters every navigation target these flags gate (genre
    // grids, artist browse) down to downloaded content — an underlined link
    // that lands on an empty grid defeats the flag's purpose, so intersect
    // here too. Mirrors the offline blocks in get_genre_tree/get_all_artists.
    if state.effective_offline() {
        let downloaded =
            super::with_cache(&state, |db| db.downloaded_album_internal_ids()).unwrap_or_default();
        genre_album_sets = genre_album_sets
            .into_iter()
            .filter_map(|(name, ids)| {
                let kept: std::collections::HashSet<i64> =
                    ids.intersection(&downloaded).copied().collect();
                if kept.is_empty() {
                    None
                } else {
                    Some((name, kept))
                }
            })
            .collect();
        let allowed =
            super::with_cache(&state, |db| db.downloaded_artist_names()).unwrap_or_default();
        artist_names.retain(|n| allowed.contains(n));
    }

    let guard = state.genre_mapper.read();
    let Some(mapper) = guard.as_ref() else {
        return Ok(None);
    };
    let Some(meta) = mapper.genre_metadata(&name) else {
        return Ok(None);
    };

    let library_genres = mapper.library_genre_names(&genre_album_sets);
    // Normalized artist name -> actual library name, so a tolerant match (e.g.
    // "blink-182" vs "Blink 182") still navigates to the real artist. Names
    // that normalize to nothing (all-punctuation bands like "!!!") get an
    // exact-lowercase key behind a NUL prefix instead — normalized keys are
    // alphanumeric-only, so the prefix can't collide, and the tokenizer falls
    // back to that slot for exact matches only (no cross-name looseness).
    let library_artists: std::collections::HashMap<String, String> = artist_names
        .iter()
        .map(|n| {
            let key = normalize_artist(n);
            if key.is_empty() {
                (format!("\u{0}{}", n.trim().to_lowercase()), n.clone())
            } else {
                (key, n.clone())
            }
        })
        .collect();

    let in_library = library_genres.contains(&meta.canonical_name.to_lowercase());
    let description_segments = match &meta.full_description {
        Some(desc) => build_description_segments(desc, &library_genres, &library_artists),
        None => Vec::new(),
    };

    Ok(Some(GenreMetadataResponse {
        canonical_name: meta.canonical_name,
        in_library,
        short_summary: meta.short_summary,
        cosmetic_aka: meta.cosmetic_aka,
        description_segments,
    }))
}

#[tauri::command]
pub async fn flush_image_cache(state: State<'_, AppState>) -> CmdResult<()> {
    state.image_cache.lock().flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_cache_stats(state: State<'_, AppState>) -> CmdResult<ImageCacheStats> {
    let cache = state.image_cache.lock();
    Ok(ImageCacheStats {
        entry_count: cache.entry_count(),
        total_size_bytes: cache.total_size(),
        pinned_count: cache.pinned_count(),
        pinned_size_bytes: cache.pinned_size(),
    })
}

#[tauri::command]
pub async fn import_custom_genres(
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<Vec<String>> {
    let (data, warnings) = CustomGenreParser::parse(&text).map_err(|e| e.to_string())?;
    let mapper = GenreMapper::from_json_bytes(&data).map_err(|e| e.to_string())?;
    ramus_core::settings::save_custom_genres(&data).map_err(|e| e.to_string())?;
    // Persist genre source preference.
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Custom;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    *state.genre_mapper.write() = Some(mapper);
    Ok(warnings)
}

#[tauri::command]
pub async fn remove_custom_genres(state: State<'_, AppState>) -> CmdResult<()> {
    // Delete custom genre file and revert to bundled open.json.
    ramus_core::settings::delete_custom_genres();
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Open;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    let open_json = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
        *state.genre_mapper.write() = Some(mapper);
    } else {
        *state.genre_mapper.write() = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_audio_cache(state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_cancel();

    // Clear the in-memory DownloadCache and collect paths to delete.
    let paths = state.player.with_cache(|cache| cache.clear());

    // Delete audio files + sibling .spec files from disk.
    for path in paths {
        let spec = spec_file_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&spec).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn get_audio_cache_stats(state: State<'_, AppState>) -> CmdResult<AudioCacheStats> {
    let (count, size) = state
        .player
        .with_cache(|cache| (cache.len(), cache.total_size()));
    Ok(AudioCacheStats {
        entry_count: count,
        total_size_bytes: size,
    })
}
