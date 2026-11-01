//! The widget's files in the shared App Group folder. Every write lands in
//! a temporary file first and is renamed into place, so the widget never
//! reads a half-written snapshot or cover.

use std::io;
use std::path::Path;

use ramus_core::widget::{WidgetPlayState, WidgetSnapshot};

pub const SNAPSHOT_FILE: &str = "snapshot.json";
pub const ART_FILE: &str = "art.jpg";

/// Whether a snapshot's face shows a cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtAction {
    Show,
    /// No cover: signed out (one account's art must not linger for the
    /// next) or nothing to resume.
    Remove,
}

pub fn art_action(snap: &WidgetSnapshot) -> ArtAction {
    match (snap.state, &snap.track) {
        (WidgetPlayState::SignedOut, _) | (_, None) => ArtAction::Remove,
        _ => ArtAction::Show,
    }
}

/// What to do with the cover file before writing `snap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtStep {
    /// The cover for `thumb` is already in place.
    Keep,
    /// Copy the cover for this thumb in.
    Copy(String),
    /// Remove the file, so a previous track's cover never shows under a
    /// track that has none of its own.
    Remove,
}

/// The cover step for `snap`, whose track's art is `thumb`, when the file
/// in place was copied from `shown`.
pub fn art_step(snap: &WidgetSnapshot, thumb: Option<&str>, shown: Option<&str>) -> ArtStep {
    match (art_action(snap), thumb) {
        (ArtAction::Show, Some(t)) if shown == Some(t) => ArtStep::Keep,
        (ArtAction::Show, Some(t)) => ArtStep::Copy(t.to_string()),
        _ => ArtStep::Remove,
    }
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, dir.join(name))
}

pub fn write_snapshot(dir: &Path, snap: &WidgetSnapshot) -> io::Result<()> {
    let json = serde_json::to_vec(snap).map_err(io::Error::other)?;
    write_atomic(dir, SNAPSHOT_FILE, &json)
}

pub fn read_snapshot(dir: &Path) -> Option<WidgetSnapshot> {
    let bytes = std::fs::read(dir.join(SNAPSHOT_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Copy `src` in as the widget's cover, or remove the cover when `None`.
pub fn write_art(dir: &Path, src: Option<&Path>) -> io::Result<()> {
    match src {
        Some(src) => write_atomic(dir, ART_FILE, &std::fs::read(src)?),
        None => match std::fs::remove_file(dir.join(ART_FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramus_core::models::PlaybackStatus;
    use ramus_core::widget::{build_snapshot, SnapshotInput, WidgetTrack};

    fn snap(signed_in: bool, with_track: bool, resume: bool) -> WidgetSnapshot {
        build_snapshot(
            SnapshotInput {
                signed_in,
                status: PlaybackStatus::Paused,
                track: with_track.then(|| WidgetTrack {
                    rating_key: "9".into(),
                    title: "Tessellate".into(),
                    artist: "alt-J".into(),
                    album: "An Awesome Wave".into(),
                    duration_sec: 182.0,
                }),
                position_sec: 3.0,
                has_queue: with_track,
                resume_enabled: resume,
                colors: None,
                waveform: None,
                cast_player: None,
                now: 10.0,
            },
            None,
        )
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let s = snap(true, true, true);
        write_snapshot(dir.path(), &s).unwrap();
        assert_eq!(read_snapshot(dir.path()), Some(s));
        assert_eq!(read_snapshot(&dir.path().join("missing")), None);
    }

    #[test]
    fn write_is_atomic_no_tmp_left() {
        let dir = tempfile::tempdir().unwrap();
        write_snapshot(dir.path(), &snap(true, true, true)).unwrap();
        let src = dir.path().join("src.jpg");
        std::fs::write(&src, b"jpeg").unwrap();
        write_art(dir.path(), Some(&src)).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![ART_FILE, SNAPSHOT_FILE, "src.jpg"]);
        assert_eq!(std::fs::read(dir.path().join(ART_FILE)).unwrap(), b"jpeg");
    }

    #[test]
    fn write_art_none_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ART_FILE), b"old").unwrap();
        write_art(dir.path(), None).unwrap();
        assert!(!dir.path().join(ART_FILE).exists());
        // Removing art that isn't there is fine.
        write_art(dir.path(), None).unwrap();
    }

    #[test]
    fn signed_out_publish_removes_art() {
        assert_eq!(art_action(&snap(false, true, true)), ArtAction::Remove);
        assert_eq!(art_action(&snap(true, false, false)), ArtAction::Remove);
        assert_eq!(art_action(&snap(true, true, true)), ArtAction::Show);
        // A paused queue that can't be resumed still shows its cover until
        // the widget goes idle.
        assert_eq!(art_action(&snap(true, true, false)), ArtAction::Show);
        assert_eq!(snap(false, true, true).state, WidgetPlayState::SignedOut);
    }

    #[test]
    fn art_step_removes_the_old_cover_when_the_track_has_none() {
        let s = snap(true, true, true);
        assert_eq!(art_step(&s, None, Some("/t/a")), ArtStep::Remove);
    }

    #[test]
    fn art_step_copies_a_new_cover_and_keeps_a_shown_one() {
        let s = snap(true, true, true);
        assert_eq!(
            art_step(&s, Some("/t/b"), Some("/t/a")),
            ArtStep::Copy("/t/b".into())
        );
        assert_eq!(
            art_step(&s, Some("/t/b"), None),
            ArtStep::Copy("/t/b".into())
        );
        assert_eq!(art_step(&s, Some("/t/a"), Some("/t/a")), ArtStep::Keep);
        assert_eq!(
            art_step(&snap(false, true, true), Some("/t/a"), Some("/t/a")),
            ArtStep::Remove
        );
    }
}
