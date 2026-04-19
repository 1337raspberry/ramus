//! Cross-platform media key handling types and trait. Platform-specific
//! integration lives in the Tauri shell layer.

use crate::models::Track;

/// Metadata for OS-level Now Playing / media key display.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub position: f64,
    pub is_playing: bool,
    pub track_number: Option<i32>,
    pub cover_url: Option<String>,
}

impl MediaMetadata {
    /// Build metadata from a Track and current playback state.
    pub fn from_track(track: &Track, position: f64, duration: f64, is_playing: bool) -> Self {
        Self {
            title: track.title.clone(),
            artist: track.display_artist().to_string(),
            album: track.album_title.clone(),
            duration,
            position,
            is_playing,
            track_number: track.index,
            cover_url: track.thumb.clone(),
        }
    }

    /// Return a copy with position and playing state replaced.
    pub fn with_playback_state(&self, position: f64, is_playing: bool) -> Self {
        Self {
            position,
            is_playing,
            ..self.clone()
        }
    }
}

/// Events received from OS-level media key controls.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaKeyEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(f64),
}

/// Platform-specific media key handler. Implemented in the Tauri shell
/// layer using `souvlaki`.
pub trait MediaKeyHandler: Send + Sync {
    /// Update Now Playing metadata on track change.
    fn update_metadata(&self, metadata: &MediaMetadata);

    /// Lightweight update for play/pause/seek without rebuilding metadata.
    fn update_playback_state(&self, is_playing: bool, position: f64);

    /// Clear Now Playing info.
    fn clear(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track() -> Track {
        Track {
            rating_key: "123".into(),
            title: "Paranoid Android".into(),
            artist_name: "Radiohead".into(),
            track_artist: None,
            album_title: "OK Computer".into(),
            album_key: Some("456".into()),
            index: Some(2),
            duration: 386.0,
            codec: Some("flac".into()),
            part_key: None,
            thumb: Some("/library/metadata/456/thumb/12345".into()),
            is_favourite: false,
            bitrate: None,
            disc_number: Some(1),
            file_size_bytes: None,
        }
    }

    #[test]
    fn test_metadata_from_track() {
        let track = make_track();
        let meta = MediaMetadata::from_track(&track, 42.5, 386.0, true);

        assert_eq!(meta.title, "Paranoid Android");
        assert_eq!(meta.artist, "Radiohead");
        assert_eq!(meta.album, "OK Computer");
        assert!((meta.duration - 386.0).abs() < 0.01);
        assert!((meta.position - 42.5).abs() < 0.01);
        assert!(meta.is_playing);
        assert_eq!(meta.track_number, Some(2));
        assert!(meta.cover_url.is_some());
    }

    #[test]
    fn test_metadata_uses_display_artist() {
        let track = Track {
            track_artist: Some("Thom Yorke".into()),
            ..make_track()
        };
        let meta = MediaMetadata::from_track(&track, 0.0, 100.0, false);
        assert_eq!(meta.artist, "Thom Yorke");
    }

    #[test]
    fn test_metadata_with_playback_state() {
        let track = make_track();
        let meta = MediaMetadata::from_track(&track, 0.0, 386.0, false);

        let updated = meta.with_playback_state(120.0, true);
        assert_eq!(updated.title, "Paranoid Android");
        assert!((updated.position - 120.0).abs() < 0.01);
        assert!(updated.is_playing);
    }

    #[test]
    fn test_metadata_no_thumb() {
        let track = Track {
            thumb: None,
            ..make_track()
        };
        let meta = MediaMetadata::from_track(&track, 0.0, 100.0, true);
        assert!(meta.cover_url.is_none());
    }

    #[test]
    fn test_metadata_no_track_number() {
        let track = Track {
            index: None,
            ..make_track()
        };
        let meta = MediaMetadata::from_track(&track, 0.0, 100.0, true);
        assert!(meta.track_number.is_none());
    }

    #[test]
    fn test_media_key_events() {
        let events = [
            MediaKeyEvent::Play,
            MediaKeyEvent::Pause,
            MediaKeyEvent::Toggle,
            MediaKeyEvent::Next,
            MediaKeyEvent::Previous,
            MediaKeyEvent::Stop,
            MediaKeyEvent::Seek(60.0),
        ];
        assert_eq!(events.len(), 7);
        assert_ne!(MediaKeyEvent::Play, MediaKeyEvent::Pause);
        assert_eq!(MediaKeyEvent::Seek(10.0), MediaKeyEvent::Seek(10.0));
        assert_ne!(MediaKeyEvent::Seek(10.0), MediaKeyEvent::Seek(20.0));
    }

    #[test]
    fn test_mock_handler_implements_trait() {
        struct MockHandler;
        impl MediaKeyHandler for MockHandler {
            fn update_metadata(&self, _metadata: &MediaMetadata) {}
            fn update_playback_state(&self, _is_playing: bool, _position: f64) {}
            fn clear(&self) {}
        }

        let handler: Box<dyn MediaKeyHandler> = Box::new(MockHandler);
        let track = make_track();
        let meta = MediaMetadata::from_track(&track, 0.0, 100.0, true);
        handler.update_metadata(&meta);
        handler.update_playback_state(false, 50.0);
        handler.clear();
    }
}
