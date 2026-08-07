//! On-disk persistence for the playback queue, so a restart resumes where
//! the previous session left off.
//!
//! Stores whole [`Track`] records rather than rating keys. That costs a
//! larger file, but it keeps restore free of any dependency on the library
//! cache DB — the queue can be adopted before the DB opens, and it survives
//! a launch with no network and no server at all. The player only needs a
//! `Track` to resolve a playback URL.
//!
//! **Split across two files, deliberately.** The track list changes rarely
//! (a new queue, an edit, a track advance) but the playing position changes
//! constantly. Keeping both in one file meant rewriting the entire track
//! list every few seconds just to record one float — at a 2000-track cap
//! that is roughly 800 KB per write, i.e. hundreds of MB of flash churn per
//! listening hour, for no reason. So:
//!
//! - [`QUEUE_FILE`] holds tracks + index, written only on structural change.
//! - [`POSITION_FILE`] holds a few dozen bytes, written on the position tick.
//!
//! The position carries the rating key it belongs to, so a snapshot left
//! over from the previous track (the queue file advanced, the position file
//! had not yet caught up) is rejected on load rather than applied to the
//! wrong track.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::Track;
use crate::plex::token_store;

const QUEUE_FILE: &str = "queue.json";
const POSITION_FILE: &str = "queue-position.json";

/// Bumped whenever the stored shape changes incompatibly. A mismatch is
/// treated as "no queue to restore" rather than migrated — the cost of
/// losing one session's queue is a single re-queue, and migration code for
/// a throwaway cache is not worth carrying.
///
/// Deliberately NOT bumped when the position moved into its own file: that
/// change only *removed* a field, so a snapshot written by the previous
/// build still deserializes (serde ignores the now-unknown `position`). The
/// queue survives the upgrade and merely restores at 0:00 once, which beats
/// discarding it outright.
const SCHEMA_VERSION: u32 = 1;

/// Upper bound on persisted tracks.
///
/// The mobile now-playing sheet renders its whole remaining queue
/// unvirtualized, so restoring a "shuffle the entire library" queue would
/// fire a full-queue art burst on launch. A window around the current index
/// keeps startup cheap while covering every realistic listening session.
pub const MAX_PERSISTED_TRACKS: usize = 2000;

/// Tracks kept *behind* the current index when windowing, so `previous`
/// still has somewhere to go after a restore.
const HISTORY_KEEP: usize = 20;

fn queue_path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(QUEUE_FILE))
}

fn position_path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(POSITION_FILE))
}

/// The track list as it exists on disk. Carries no position — see the module
/// note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredQueue {
    version: u32,
    tracks: Vec<Track>,
    index: usize,
}

impl Default for StoredQueue {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            tracks: Vec::new(),
            index: 0,
        }
    }
}

/// The playing position as it exists on disk, tagged with the track it was
/// measured against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredPosition {
    version: u32,
    rating_key: String,
    position: f64,
}

impl Default for StoredPosition {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            rating_key: String::new(),
            position: 0.0,
        }
    }
}

/// A restored queue: the two files merged and validated.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedQueue {
    pub tracks: Vec<Track>,
    pub index: usize,
    pub position: f64,
}

/// Reduce a queue to at most [`MAX_PERSISTED_TRACKS`] entries centred on the
/// playing track, returning the trimmed tracks and the remapped index.
///
/// Split out from [`save`] so it can be tested without touching the disk.
pub fn window(tracks: &[Track], index: usize) -> (Vec<Track>, usize) {
    if tracks.len() <= MAX_PERSISTED_TRACKS {
        return (tracks.to_vec(), index);
    }
    // Keep a little history behind the playing track, then fill forward —
    // upcoming tracks are what a resumed session actually needs. Clamp the
    // start so a queue near its end still yields a full window.
    let max_start = tracks.len() - MAX_PERSISTED_TRACKS;
    let start = index.saturating_sub(HISTORY_KEEP).min(max_start);
    let end = (start + MAX_PERSISTED_TRACKS).min(tracks.len());
    (tracks[start..end].to_vec(), index.saturating_sub(start))
}

/// Atomically replace `path` with `bytes`.
///
/// Write-then-rename because these files are rewritten during playback and a
/// process death mid-write would otherwise leave a truncated one. That only
/// costs a session's queue (a torn file fails to parse and restore is
/// skipped), but rename is atomic on both POSIX and NTFS and the guarantee
/// is three lines.
fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn sane_position(position: f64) -> f64 {
    if position.is_finite() {
        position.max(0.0)
    } else {
        0.0
    }
}

/// Write both files: the windowed track list and the position.
///
/// For the structural saves (new queue, edit, track advance, shutdown
/// flush). The periodic position tick uses [`save_position`] instead, which
/// leaves the much larger track list alone.
pub fn save(tracks: &[Track], index: usize, position: f64) -> Result<(), String> {
    if tracks.is_empty() || index >= tracks.len() {
        clear();
        return Ok(());
    }
    let path = queue_path().ok_or("no config directory available")?;

    let rating_key = tracks[index].rating_key.clone();
    let (windowed, index) = window(tracks, index);
    let snapshot = StoredQueue {
        version: SCHEMA_VERSION,
        tracks: windowed,
        index,
    };
    let json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    write_atomic(&path, &json)?;

    // Keep the two in step: a structural save that left a stale position
    // behind would have it rejected on load (rating key mismatch) and lose a
    // position the user is still listening at.
    save_position(&rating_key, position)
}

/// Write only the position, tagged with the track it belongs to.
///
/// This is the one that runs on a timer, so it must stay small — see the
/// module note on why the track list is not rewritten here.
pub fn save_position(rating_key: &str, position: f64) -> Result<(), String> {
    let path = position_path().ok_or("no config directory available")?;
    let snapshot = StoredPosition {
        version: SCHEMA_VERSION,
        rating_key: rating_key.to_string(),
        position: sane_position(position),
    };
    let json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    write_atomic(&path, &json)
}

/// Read the stored queue, or `None` when there is nothing usable to restore.
///
/// Every failure mode — missing file, torn write, hand-edited JSON, a
/// snapshot from an incompatible version, an index pointing past the end —
/// collapses to `None`, i.e. an ordinary cold launch. A missing or
/// mismatched position file is milder still: the queue restores at 0:00.
pub fn load() -> Option<PersistedQueue> {
    let path = queue_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let stored: StoredQueue = serde_json::from_slice(&bytes).ok()?;

    if stored.version != SCHEMA_VERSION {
        log::info!(
            "queue_store: ignoring snapshot from schema version {}",
            stored.version
        );
        return None;
    }
    if stored.tracks.is_empty() || stored.index >= stored.tracks.len() {
        return None;
    }

    let position = load_position()
        .filter(|p| p.rating_key == stored.tracks[stored.index].rating_key)
        .map(|p| sane_position(p.position))
        .unwrap_or(0.0);

    Some(PersistedQueue {
        tracks: stored.tracks,
        index: stored.index,
        position,
    })
}

fn load_position() -> Option<StoredPosition> {
    let bytes = std::fs::read(position_path()?).ok()?;
    let stored: StoredPosition = serde_json::from_slice(&bytes).ok()?;
    (stored.version == SCHEMA_VERSION && !stored.rating_key.is_empty()).then_some(stored)
}

/// Delete the stored queue. Called when the queue is cleared and on sign-out
/// (one account's queue must never surface under the next).
pub fn clear() {
    for path in [queue_path(), position_path()].into_iter().flatten() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: format!("Track {key}"),
            artist_name: "Artist".to_string(),
            track_artist: None,
            album_title: "Album".to_string(),
            album_key: Some("album-1".to_string()),
            index: None,
            duration: 180.0,
            codec: Some("flac".to_string()),
            part_key: Some(format!("/library/parts/{key}/file.flac")),
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count: None,
        }
    }

    fn tracks(n: usize) -> Vec<Track> {
        (0..n).map(|i| track(&i.to_string())).collect()
    }

    #[test]
    fn test_snapshot_round_trips_through_json() {
        let snapshot = StoredQueue {
            version: SCHEMA_VERSION,
            tracks: tracks(3),
            index: 1,
        };
        let json = serde_json::to_vec(&snapshot).unwrap();
        let parsed: StoredQueue = serde_json::from_slice(&json).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn test_position_snapshot_round_trips_and_stays_small() {
        let snapshot = StoredPosition {
            version: SCHEMA_VERSION,
            rating_key: "73319".into(),
            position: 64.4,
        };
        let json = serde_json::to_vec(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_slice::<StoredPosition>(&json).unwrap(),
            snapshot
        );
        // The whole point of the split: this is what gets rewritten on the
        // position tick, so it must stay a rounding error next to the track
        // list (which runs to hundreds of KB at the cap).
        assert!(
            json.len() < 256,
            "position snapshot should be tiny, got {} bytes",
            json.len()
        );
    }

    #[test]
    fn test_a_queue_file_from_the_previous_single_file_build_still_parses() {
        // The split only *removed* a field, so an old snapshot must still
        // restore its tracks — losing the position once beats discarding
        // the user's queue on upgrade.
        let legacy = br#"{"version":1,"tracks":[],"index":0,"position":42.5}"#;
        let parsed: StoredQueue = serde_json::from_slice(legacy).unwrap();
        assert_eq!(parsed.version, SCHEMA_VERSION);
        assert_eq!(parsed.index, 0);
    }

    #[test]
    fn test_short_queue_is_stored_whole() {
        let all = tracks(10);
        let (windowed, idx) = window(&all, 4);
        assert_eq!(windowed.len(), 10);
        assert_eq!(idx, 4);
    }

    #[test]
    fn test_long_queue_windows_around_the_current_index() {
        let all = tracks(5000);
        let (windowed, idx) = window(&all, 3000);
        assert_eq!(windowed.len(), MAX_PERSISTED_TRACKS);
        // The playing track must survive the trim at its remapped index —
        // restoring the wrong track would be worse than not restoring.
        assert_eq!(windowed[idx].rating_key, all[3000].rating_key);
        assert_eq!(idx, HISTORY_KEEP);
    }

    #[test]
    fn test_windowing_near_the_queue_start_keeps_the_index() {
        let all = tracks(5000);
        let (windowed, idx) = window(&all, 5);
        assert_eq!(windowed.len(), MAX_PERSISTED_TRACKS);
        assert_eq!(idx, 5);
        assert_eq!(windowed[idx].rating_key, all[5].rating_key);
    }

    #[test]
    fn test_windowing_near_the_queue_end_still_fills_the_window() {
        let all = tracks(5000);
        let (windowed, idx) = window(&all, 4995);
        assert_eq!(windowed.len(), MAX_PERSISTED_TRACKS);
        assert_eq!(windowed[idx].rating_key, all[4995].rating_key);
    }

    #[test]
    fn test_garbage_payload_does_not_parse() {
        assert!(serde_json::from_slice::<StoredQueue>(b"{not json").is_err());
        // A truncated write — the shape `load` guards against with tmp+rename.
        assert!(serde_json::from_slice::<StoredQueue>(b"{\"version\":1,\"tra").is_err());
        assert!(serde_json::from_slice::<StoredPosition>(b"{\"version\":1,\"rat").is_err());
    }

    #[test]
    fn test_missing_fields_fall_back_to_defaults() {
        // `#[serde(default)]` means a snapshot written by an older build with
        // fewer fields still parses; the emptiness check in `load` then
        // rejects it rather than restoring a bogus queue.
        let parsed: StoredQueue = serde_json::from_slice(b"{\"version\":1}").unwrap();
        assert_eq!(parsed.version, SCHEMA_VERSION);
        assert!(parsed.tracks.is_empty());
        assert_eq!(parsed.index, 0);
    }

    #[test]
    fn test_position_is_only_applied_to_the_track_it_was_measured_on() {
        // The staleness guard. `load` pairs the files by rating key, so a
        // position left over from the previous track — the queue file
        // advanced, the 10s position tick had not yet caught up — must be
        // dropped rather than applied at the wrong offset.
        let queue = tracks(3);
        let matching = StoredPosition {
            version: SCHEMA_VERSION,
            rating_key: queue[1].rating_key.clone(),
            position: 64.4,
        };
        let stale = StoredPosition {
            version: SCHEMA_VERSION,
            rating_key: queue[0].rating_key.clone(),
            position: 999.0,
        };
        let applies = |p: &StoredPosition| p.rating_key == queue[1].rating_key;
        assert!(applies(&matching));
        assert!(!applies(&stale));
    }

    #[test]
    fn test_non_finite_positions_are_neutralised() {
        assert_eq!(sane_position(f64::NAN), 0.0);
        assert_eq!(sane_position(f64::INFINITY), 0.0);
        assert_eq!(sane_position(-5.0), 0.0);
        assert_eq!(sane_position(64.4), 64.4);
    }
}
