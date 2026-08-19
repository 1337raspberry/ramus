//! Generated-playlist ("crate") commands.
//!
//! A crate applies rules Plex's smart-playlist grammar can't express — picking
//! an album's most popular tracks, and matching a genre's whole subtree — so
//! the result is materialised as an ordinary playlist rather than stored as a
//! server-side filter. The recipe lives in the playlist's `summary`, which is
//! what makes a crate regenerable from any device instead of only the one that
//! built it.
//!
//! Selection itself is `ramus_core::crates`; this layer only resolves genres to
//! candidate tracks and talks to the server.

use std::collections::HashSet;

use serde::Serialize;
use tauri::State;

use ramus_core::crates::{eligible_tracks, CrateRecipe};
use ramus_core::models::{Playlist, PlaylistItem, Track};
use ramus_core::plex::client::build_library_uri;
use ramus_core::search::engine::GenreExpander;

use crate::state::AppState;

use super::playlists::{get_machine_identifier, refresh_items, to_upsert_row};
use super::{with_cache, CmdResult};

/// What a recipe would produce, without creating anything.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateEstimate {
    /// Library genre tags the recipe resolves to (after sub-genre expansion).
    pub genre_tags: usize,
    /// Tracks on albums carrying any of those tags, before any rule runs.
    pub candidate_tracks: usize,
    /// Distinct albums behind those candidates.
    pub candidate_albums: usize,
    /// Tracks left once the rules have run — the pool the crate draws from.
    /// This is the figure that responds to the per-album and unplayed
    /// settings; `candidate_tracks` only moves with the genre selection.
    pub eligible_tracks: usize,
    /// How many tracks the crate would actually hold — below the requested
    /// count when the rules don't leave enough to fill it.
    pub selected: usize,
    /// The human-readable sentence that will head the playlist summary.
    pub description: String,
    /// The name the playlist will take. Crate titles are derived from the
    /// recipe rather than typed, so the preview shows the name live.
    pub derived_title: String,
}

/// Expand a recipe's genres through the genre tree and keep only tags the
/// library actually has.
///
/// The fuzzy leg of `expand_genre` is loose enough to land on a neighbouring
/// family, so an expansion that doesn't contain the original name is
/// discarded in favour of the raw name — the same guard every other caller
/// applies.
fn resolve_genre_tags(state: &State<'_, AppState>, recipe: &CrateRecipe) -> CmdResult<Vec<String>> {
    let mut wanted: HashSet<String> = HashSet::new();
    {
        let mapper_guard = state.genre_mapper.read();
        for genre in &recipe.genres {
            let lower = genre.to_lowercase();
            if !recipe.include_subgenres {
                wanted.insert(lower);
                continue;
            }
            match mapper_guard.as_ref().and_then(|m| m.expand_genre(genre)) {
                Some(set) if set.iter().any(|n| n.eq_ignore_ascii_case(genre)) => {
                    wanted.extend(set.into_iter().map(|s| s.to_lowercase()));
                }
                _ => {
                    wanted.insert(lower);
                }
            }
        }
    }

    // Intersect with the library's own tags: the tree carries thousands of
    // genres the user has no music for, and every one sent would be a value
    // the server has to match for nothing.
    let tags: Vec<String> = with_cache(state, |db| db.genre_album_sets())?
        .into_keys()
        .filter(|tag| wanted.contains(&tag.to_lowercase()))
        .collect();
    Ok(tags)
}

/// Candidate tracks for a recipe: every track on an album carrying any of the
/// resolved genre tags. Whole albums, because ranking a track against its
/// siblings needs the siblings.
fn resolve_candidates(
    state: &State<'_, AppState>,
    recipe: &CrateRecipe,
) -> CmdResult<(Vec<String>, Vec<Track>)> {
    let tags = resolve_genre_tags(state, recipe)?;
    if tags.is_empty() {
        return Ok((tags, Vec::new()));
    }
    let refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tracks = with_cache(state, |db| db.tracks_for_genres(&refs))?;
    Ok((tags, tracks))
}

/// Preview a recipe's output without creating a playlist.
#[tauri::command]
pub async fn estimate_crate(state: State<'_, AppState>, recipe: CrateRecipe) -> CmdResult<CrateEstimate> {
    let (tags, candidates) = resolve_candidates(&state, &recipe)?;
    // Count before selecting so the candidates can be moved into it rather
    // than cloned — a broad genre reaches five figures of tracks, and this
    // runs on every edit to the form.
    let candidate_tracks = candidates.len();
    let candidate_albums = candidates
        .iter()
        .filter_map(|t| t.album_key.as_deref())
        .collect::<HashSet<_>>()
        .len();

    // Size the pool rather than building a crate: the shuffle can't change a
    // count, and this runs on every edit to the form.
    let eligible = eligible_tracks(candidates, &recipe).len();
    let selected = if recipe.count > 0 {
        eligible.min(recipe.count as usize)
    } else {
        eligible
    };

    Ok(CrateEstimate {
        genre_tags: tags.len(),
        candidate_tracks,
        candidate_albums,
        eligible_tracks: eligible,
        selected,
        description: recipe.describe(),
        derived_title: recipe.derived_title(),
    })
}

/// Build a crate and store it as an ordinary playlist, with its recipe in the
/// summary so it can be regenerated later.
#[tauri::command]
pub async fn create_crate_playlist(
    state: State<'_, AppState>,
    recipe: CrateRecipe,
) -> CmdResult<Playlist> {
    if recipe.genres.is_empty() {
        return Err("Pick a genre".into());
    }
    let title = recipe.derived_title();

    let (_, candidates) = resolve_candidates(&state, &recipe)?;
    let picked = recipe.select(candidates);
    if picked.is_empty() {
        return Err("Nothing matched those rules".into());
    }

    let track_ids: Vec<String> = picked.iter().map(|t| t.rating_key.clone()).collect();
    let machine_id = get_machine_identifier()?;
    let uri = build_library_uri(&machine_id, &track_ids);
    let created = state
        .client
        .create_playlist(&title, &uri)
        .await
        .map_err(|e| e.to_string())?;

    // The recipe is what makes this a crate rather than a frozen list, so a
    // failure to attach it is fatal to the feature even though the playlist
    // itself now exists. Surface it instead of silently creating something
    // that can never be regenerated.
    let summary = recipe.to_summary();
    state
        .client
        .set_playlist_summary(&created.rating_key, &summary)
        .await
        .map_err(|e| format!("Playlist created, but its recipe couldn't be saved: {e}"))?;

    let mut row = to_upsert_row(&created);
    row.summary = Some(summary.clone());
    with_cache(&state, |db| db.upsert_playlist(&row))?;
    let _ = refresh_items(&state, &created.rating_key).await;

    Ok(Playlist {
        source_id: created.rating_key.clone(),
        title: created.title.clone(),
        smart: false,
        track_count: Some(track_ids.len() as i64),
        duration: created.duration.map(|ms| ms as f64 / 1000.0),
        thumb: created.composite.clone(),
        summary: Some(summary),
        is_crate: true,
    })
}

/// A playlist's recipe, or `None` when it isn't a crate (or its summary has
/// since been rewritten). Drives whether the UI offers Regenerate.
#[tauri::command]
pub async fn get_crate_recipe(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Option<CrateRecipe>> {
    Ok(load_recipe(&state, &source_id).await)
}

/// Read a playlist's recipe, preferring the mirror and falling back to a
/// detail fetch. The list endpoint doesn't reliably carry `summary`, so a
/// mirror populated only from a listing can legitimately lack it.
async fn load_recipe(state: &State<'_, AppState>, source_id: &str) -> Option<CrateRecipe> {
    // A stored summary is authoritative, including when it holds no recipe:
    // that's how an ordinary playlist avoids paying for a lookup every time
    // it's opened. Only a row that has never seen one falls through.
    let mirrored = with_cache(state, |db| db.all_playlists())
        .ok()?
        .into_iter()
        .find(|p| p.source_id == source_id)
        .and_then(|p| p.summary);
    if let Some(summary) = mirrored {
        return CrateRecipe::from_summary(&summary);
    }

    let detail = state.client.playlist_detail(source_id).await.ok()?;
    let summary = detail.summary.clone().unwrap_or_default();

    // Backfill either way — caching "this isn't a crate" is what stops the
    // next open repeating the round trip. Empty stays non-null so the
    // upsert's COALESCE treats it as known rather than missing.
    let mut row = to_upsert_row(&detail);
    row.summary = Some(summary.clone());
    let _ = with_cache(state, |db| db.upsert_playlist(&row));

    CrateRecipe::from_summary(&summary)
}

/// Re-run a crate's recipe against the current library and replace its tracks.
#[tauri::command]
pub async fn regenerate_crate_playlist(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<PlaylistItem>> {
    let recipe = load_recipe(&state, &source_id)
        .await
        .ok_or("This playlist has no ramus recipe to regenerate from")?;
    rebuild_playlist(&state, &source_id, &recipe).await
}

/// A saved edit's result: the renamed/re-summarised playlist plus its fresh
/// tracks, so the open detail view can repaint without a refetch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateUpdate {
    pub playlist: Playlist,
    pub items: Vec<PlaylistItem>,
}

/// Change a crate's recipe in place: store the new recipe, regenerate the
/// tracks under it, and rename the playlist to the new derived title. The
/// title is derived rather than kept, so an edit that changes the rules
/// renames the crate to keep the name honest.
#[tauri::command]
pub async fn update_crate_playlist(
    state: State<'_, AppState>,
    source_id: String,
    recipe: CrateRecipe,
) -> CmdResult<CrateUpdate> {
    if recipe.genres.is_empty() {
        return Err("Pick a genre".into());
    }

    // Write the recipe first: if anything later fails, the stored recipe
    // already matches what the user asked for, and a plain Regenerate
    // finishes the job.
    let summary = recipe.to_summary();
    state
        .client
        .set_playlist_summary(&source_id, &summary)
        .await
        .map_err(|e| format!("Couldn't save the new recipe: {e}"))?;
    // Mirror it immediately: the recipe loader trusts the mirror without
    // refetching, so if the rebuild below fails part-way, a plain Regenerate
    // must already see the new rules.
    with_cache(&state, |db| db.set_playlist_summary(&source_id, &summary))?;

    let items = rebuild_playlist(&state, &source_id, &recipe).await?;

    let title = recipe.derived_title();
    state
        .client
        .rename_playlist(&source_id, &title)
        .await
        .map_err(|e| format!("The crate was rebuilt but couldn't be renamed: {e}"))?;

    // Refresh the mirror row from the server so title, summary, count and
    // duration all land together; the detail fetch carries the summary the
    // listing endpoint omits.
    if let Ok(detail) = state.client.playlist_detail(&source_id).await {
        let mut row = to_upsert_row(&detail);
        row.summary = Some(detail.summary.clone().unwrap_or_else(|| summary.clone()));
        with_cache(&state, |db| db.upsert_playlist(&row))?;
    } else {
        with_cache(&state, |db| db.rename_playlist(&source_id, &title))?;
    }

    let playlist = with_cache(&state, |db| db.all_playlists())?
        .into_iter()
        .find(|p| p.source_id == source_id)
        .ok_or_else(|| "Playlist not found".to_string())?;

    Ok(CrateUpdate { playlist, items })
}

/// Select tracks for a recipe and replace the playlist's contents with them.
async fn rebuild_playlist(
    state: &State<'_, AppState>,
    source_id: &str,
    recipe: &CrateRecipe,
) -> CmdResult<Vec<PlaylistItem>> {
    let (_, candidates) = resolve_candidates(state, recipe)?;
    let picked = recipe.select(candidates);
    if picked.is_empty() {
        return Err("Nothing matched those rules — the playlist was left alone".into());
    }

    let existing = state
        .client
        .playlist_items(source_id)
        .await
        .map_err(|e| e.to_string())?;
    let stale_ids: Vec<i64> = existing.iter().filter_map(|m| m.playlist_item_id).collect();

    // Clear before adding, even though the reverse order would survive a
    // part-way failure better. Plex silently drops an added track that the
    // playlist already holds, so adding first means any track common to both
    // the old and new selection is never re-added — and removing its old
    // entry afterwards then deletes it outright, leaving the crate short by the
    // size of the overlap. Emptying first makes every add land.
    //
    // There is no bulk clear, so this is N removals; a failure between the
    // two halves leaves the playlist empty, which another regenerate fixes.
    for item_id in stale_ids {
        state
            .client
            .remove_playlist_item(source_id, item_id)
            .await
            .map_err(|e| e.to_string())?;
    }

    let track_ids: Vec<String> = picked.iter().map(|t| t.rating_key.clone()).collect();
    let machine_id = get_machine_identifier()?;
    let uri = build_library_uri(&machine_id, &track_ids);
    state
        .client
        .add_playlist_items(source_id, &uri)
        .await
        .map_err(|e| {
            format!("The playlist was cleared but the new tracks couldn't be added ({e}) — regenerate again to retry")
        })?;

    refresh_items(state, source_id).await
}
