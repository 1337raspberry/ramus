use super::*;
use crate::playback::mpv::MpvPlayer;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields read via Debug/pattern matching in assertions
enum MockCall {
    LoadFile {
        url: String,
        mode: LoadMode,
        options: Option<String>,
    },
    LoadFileAt {
        url: String,
        index: i64,
        options: Option<String>,
    },
    PlaylistPlayIndex(i64),
    PlaylistRemove(i64),
    PlaylistMove { from: i64, to: i64 },
    Seek(f64),
    SetPause(bool),
    SetVolume(f64),
    SetAudioFilters(String),
    Stop,
}

struct MockMpv {
    calls: Mutex<Vec<MockCall>>,
    volume: Mutex<f64>,
    shutdown: AtomicBool,
}

impl MockMpv {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            volume: Mutex::new(100.0),
            shutdown: AtomicBool::new(false),
        }
    }

    fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().clone()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().len()
    }
}

impl MpvPlayer for MockMpv {
    fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>) {
        self.calls.lock().push(MockCall::LoadFile {
            url: url.to_string(),
            mode,
            options: options.map(|s| s.to_string()),
        });
    }
    fn load_file_at(&self, url: &str, index: i64, options: Option<&str>) {
        self.calls.lock().push(MockCall::LoadFileAt {
            url: url.to_string(),
            index,
            options: options.map(|s| s.to_string()),
        });
    }
    fn playlist_play_index(&self, index: i64) {
        self.calls.lock().push(MockCall::PlaylistPlayIndex(index));
    }
    fn playlist_remove(&self, index: i64) {
        self.calls.lock().push(MockCall::PlaylistRemove(index));
    }
    fn playlist_move(&self, from: i64, to: i64) {
        self.calls.lock().push(MockCall::PlaylistMove { from, to });
    }
    fn seek(&self, position: f64) {
        self.calls.lock().push(MockCall::Seek(position));
    }
    fn set_pause(&self, paused: bool) {
        self.calls.lock().push(MockCall::SetPause(paused));
    }
    fn set_volume(&self, volume: f64) {
        *self.volume.lock() = volume;
        self.calls.lock().push(MockCall::SetVolume(volume));
    }
    fn get_volume(&self) -> f64 {
        *self.volume.lock()
    }
    fn set_audio_filters(&self, value: &str) {
        self.calls
            .lock()
            .push(MockCall::SetAudioFilters(value.to_string()));
    }
    fn stop(&self) {
        self.calls.lock().push(MockCall::Stop);
    }
    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}

fn make_test_track(key: &str) -> Track {
    Track {
        rating_key: key.into(),
        title: format!("Track {key}"),
        artist_name: "Test Artist".into(),
        track_artist: None,
        album_title: "Test Album".into(),
        album_key: None,
        index: None,
        duration: 180.0,
        codec: Some("flac".into()),
        part_key: Some(format!("/library/parts/{key}/file.flac")),
        thumb: None,
        is_favourite: false,
        bitrate: None,
        disc_number: None,
        file_size_bytes: None,
        rating_count: None,
    }
}

fn make_player() -> (AudioPlayer, Arc<MockMpv>) {
    let mpv = Arc::new(MockMpv::new());
    let player = AudioPlayer::new(mpv.clone());
    player.configure(
        Url::parse("http://test.local:32400").unwrap(),
        "test-token".into(),
        "test-client".into(),
    );
    (player, mpv)
}

#[test]
fn test_eq_filter_string_all_zeros() {
    let bands = [0.0f32; 10];
    let filter = build_eq_filter_string(&bands);
    assert!(filter.starts_with("lavfi=["));
    assert!(filter.ends_with(']'));
    assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
    assert!(filter.contains("equalizer=f=16000:width_type=o:w=1:g=0.0"));
    assert_eq!(filter.matches("equalizer=").count(), 10);
}

#[test]
fn test_eq_filter_string_with_gains() {
    let bands = [3.5, -2.0, 0.0, 1.0, -1.5, 6.0, -12.0, 12.0, 0.5, -0.5];
    let filter = build_eq_filter_string(&bands);
    assert!(filter.contains("g=3.5"));
    assert!(filter.contains("g=-2.0"));
    assert!(filter.contains("g=6.0"));
    assert!(filter.contains("g=-12.0"));
    assert!(filter.contains("g=12.0"));
}

#[test]
fn test_eq_filter_string_decimal_point_not_comma() {
    let bands = [3.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let filter = build_eq_filter_string(&bands);
    assert!(filter.contains("3.5"));
    assert!(!filter.contains("3,5"));
}

#[test]
fn test_eq_filter_string_sanitizes_nan() {
    let mut bands = [0.0f32; 10];
    bands[0] = f32::NAN;
    let filter = build_eq_filter_string(&bands);
    assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
}

#[test]
fn test_eq_filter_string_sanitizes_inf() {
    let mut bands = [0.0f32; 10];
    bands[0] = f32::INFINITY;
    bands[1] = f32::NEG_INFINITY;
    let filter = build_eq_filter_string(&bands);
    assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
    assert!(filter.contains("equalizer=f=62:width_type=o:w=1:g=0.0"));
}

#[test]
fn test_eq_frequencies_count() {
    assert_eq!(EQ_FREQUENCIES.len(), 10);
    assert_eq!(EQ_FREQUENCIES[0], 31);
    assert_eq!(EQ_FREQUENCIES[9], 16000);
}

#[test]
fn test_sanitize_filename_keeps_safe_chars() {
    assert_eq!(sanitize_filename("abc123_test-file"), "abc123_test-file");
}

#[test]
fn test_sanitize_filename_strips_unsafe_chars() {
    assert_eq!(sanitize_filename("track/with:bad*chars"), "trackwithbadchars");
    assert_eq!(sanitize_filename("../../../etc/passwd"), "etcpasswd");
    assert_eq!(sanitize_filename("file name.flac"), "filenameflac");
}

#[test]
fn test_sanitize_filename_empty() {
    assert_eq!(sanitize_filename(""), "");
    assert_eq!(sanitize_filename("***"), "");
}

#[test]
fn test_allowed_extension() {
    assert!(is_allowed_extension("flac"));
    assert!(is_allowed_extension("FLAC"));
    assert!(is_allowed_extension("mp3"));
    assert!(is_allowed_extension("aac"));
    assert!(is_allowed_extension("wav"));
    assert!(is_allowed_extension("ogg"));
    assert!(is_allowed_extension("opus"));
    assert!(is_allowed_extension("m4a"));
    assert!(is_allowed_extension("bin"));
    assert!(!is_allowed_extension("exe"));
    assert!(!is_allowed_extension("sh"));
    assert!(!is_allowed_extension(""));
}

#[test]
fn test_load_queue() {
    let (player, mpv) = make_player();
    let tracks = vec![make_test_track("1"), make_test_track("2"), make_test_track("3")];

    player.load_queue(tracks.clone(), 0);

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.queue.len(), 3);
    assert_eq!(state.queue_index, 0);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "1");

    let calls = mpv.calls();
    let load_files: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::LoadFile { .. }))
        .collect();
    assert_eq!(load_files.len(), 3);

    assert!(matches!(load_files[0], MockCall::LoadFile { mode: LoadMode::Replace, .. }));
    assert!(matches!(load_files[1], MockCall::LoadFile { mode: LoadMode::Append, .. }));
    assert!(matches!(load_files[2], MockCall::LoadFile { mode: LoadMode::Append, .. }));
}

#[test]
fn test_load_queue_at_index() {
    let (player, mpv) = make_player();
    let tracks = vec![make_test_track("1"), make_test_track("2"), make_test_track("3")];

    player.load_queue(tracks, 2);

    let state = player.state();
    assert_eq!(state.queue_index, 2);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");

    let calls = mpv.calls();
    assert!(calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(2))));
}

#[test]
fn test_pos_change_to_zero_after_start_at_is_suppressed() {
    // load_queue with start_at > 0 issues `loadfile Replace` for queue[0],
    // which makes mpv fire playlist-pos-change(0) before the explicit
    // playlist_play_index lands. That transient event must not mutate
    // current_track or queue_index away from the requested start.
    let (player, _) = make_player();
    let tracks = vec![
        make_test_track("A"),
        make_test_track("B"),
        make_test_track("C"),
    ];

    player.load_queue(tracks, 2);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "C");

    // Transient pos=0 event from mpv: must be ignored (not an advance).
    assert!(!player.handle_playlist_pos_change(0));
    assert_eq!(
        player.state().current_track.as_ref().unwrap().rating_key,
        "C",
        "transient pos=0 must not flip current_track"
    );
    assert_eq!(player.state().queue_index, 2);

    // Real pos=2 event arrives; gate clears, state stays consistent.
    assert!(player.handle_playlist_pos_change(2));
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "C");

    // Subsequent natural advance to pos=0 (e.g. user clicks back to start)
    // is now processed normally because the gate cleared.
    assert!(player.handle_playlist_pos_change(0));
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "A");
    assert_eq!(player.state().queue_index, 0);
}

#[test]
fn test_load_queue_empty_is_noop() {
    let (player, mpv) = make_player();
    let initial_count = mpv.call_count();
    player.load_queue(vec![], 0);
    assert_eq!(player.state().status, PlaybackStatus::Stopped);
    assert_eq!(mpv.call_count(), initial_count);
}

#[test]
fn test_load_queue_out_of_bounds_is_noop() {
    let (player, mpv) = make_player();
    let initial_count = mpv.call_count();
    player.load_queue(vec![make_test_track("1")], 5);
    assert_eq!(player.state().status, PlaybackStatus::Stopped);
    assert_eq!(mpv.call_count(), initial_count);
}

#[test]
fn test_append_to_queue() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    let initial_calls = mpv.call_count();

    player.append_to_queue(vec![make_test_track("2"), make_test_track("3")]);

    let state = player.state();
    assert_eq!(state.queue.len(), 3);
    assert_eq!(state.queue_index, 0);

    let new_calls = &mpv.calls()[initial_calls..];
    let appends: Vec<_> = new_calls
        .iter()
        .filter(|c| matches!(c, MockCall::LoadFile { mode: LoadMode::Append, .. }))
        .collect();
    assert_eq!(appends.len(), 2);
}

#[test]
fn test_append_to_queue_auto_start() {
    let (player, _mpv) = make_player();
    player.append_to_queue(vec![make_test_track("1"), make_test_track("2")]);

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.queue.len(), 2);
    assert_eq!(state.queue_index, 0);
}

#[test]
fn test_insert_next() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("3")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.insert_next(vec![make_test_track("2")]);

    let state = player.state();
    assert_eq!(state.queue.len(), 3);
    assert_eq!(state.queue[1].rating_key, "2");
    assert_eq!(state.queue[2].rating_key, "3");

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::LoadFileAt { index: 1, .. })));
}

#[test]
fn test_insert_next_when_stopped_becomes_load() {
    let (player, _mpv) = make_player();
    player.insert_next(vec![make_test_track("1")]);

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.queue.len(), 1);
}

#[test]
fn test_remove_from_queue() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.remove_from_queue(2);

    let state = player.state();
    assert_eq!(state.queue.len(), 2);
    assert_eq!(state.queue_index, 0);

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistRemove(2))));
}

#[test]
fn test_remove_current_track_is_noop() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.remove_from_queue(0);

    assert_eq!(player.state().queue.len(), 2);
    assert_eq!(mpv.call_count(), initial_calls);
}

#[test]
fn test_remove_before_current_adjusts_index() {
    let (player, _mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        1,
    );

    player.remove_from_queue(0);

    let state = player.state();
    assert_eq!(state.queue_index, 0);
    assert_eq!(state.queue.len(), 2);
}

#[test]
fn test_jump_to_index() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.jump_to_index(2);

    let state = player.state();
    assert_eq!(state.queue_index, 2);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(2))));
}

#[test]
fn test_next() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.next();

    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(1))));
}

#[test]
fn test_next_at_end_stops() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    let initial_calls = mpv.call_count();

    player.next();

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls.iter().any(|c| matches!(c, MockCall::Stop)));
}

#[test]
fn test_previous_restarts_if_past_threshold() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        1,
    );
    player.handle_position_change(5.0);
    let initial_calls = mpv.call_count();

    player.previous();

    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(player.position(), 0.0);

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::Seek(pos) if *pos == 0.0)));
}

#[test]
fn test_previous_goes_back_if_within_threshold() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        1,
    );
    player.handle_position_change(1.0);
    let initial_calls = mpv.call_count();

    player.previous();

    let state = player.state();
    assert_eq!(state.queue_index, 0);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "1");

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(0))));
}

#[test]
fn test_previous_at_start_seeks_to_zero() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(1.0);
    let initial_calls = mpv.call_count();

    player.previous();

    assert_eq!(player.state().queue_index, 0);
    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::Seek(pos) if *pos == 0.0)));
}

#[test]
fn test_toggle_play_pause() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    player.toggle_play_pause();
    assert_eq!(player.state().status, PlaybackStatus::Paused);

    player.toggle_play_pause();
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    let calls = mpv.calls();
    let pause_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::SetPause(_)))
        .collect();
    assert!(pause_calls.len() >= 2);
}

#[test]
fn test_toggle_when_stopped_is_noop() {
    let (player, mpv) = make_player();
    let initial_calls = mpv.call_count();
    player.toggle_play_pause();
    assert_eq!(mpv.call_count(), initial_calls);
}

#[test]
fn test_seek() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_duration_change(180.0);
    let initial_calls = mpv.call_count();

    player.seek(60.0);

    assert!((player.position() - 60.0).abs() < 0.1);
    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls
        .iter()
        .any(|c| matches!(c, MockCall::Seek(pos) if (*pos - 60.0).abs() < 0.1)));
}

#[test]
fn test_seek_clamps_to_bounds() {
    let (player, _mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_duration_change(180.0);

    player.seek(-10.0);
    assert!(player.position() >= 0.0);

    player.seek(999.0);
    assert!(player.position() <= 179.5);
}

#[test]
fn test_set_volume() {
    let (player, mpv) = make_player();
    player.set_volume(75.0);

    assert!((player.volume() - 75.0).abs() < 0.1);
    let calls = mpv.calls();
    assert!(calls
        .iter()
        .any(|c| matches!(c, MockCall::SetVolume(v) if (*v - 75.0).abs() < 0.1)));
}

#[test]
fn test_set_volume_clamps() {
    let (player, _mpv) = make_player();
    player.set_volume(150.0);
    assert!((player.volume() - 100.0).abs() < 0.1);

    player.set_volume(-10.0);
    assert!(player.volume() >= 0.0);
}

#[test]
fn test_stop() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        0,
    );
    let initial_calls = mpv.call_count();

    player.stop();

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());
    assert!(state.queue.is_empty());
    assert_eq!(state.queue_index, 0);

    let new_calls = &mpv.calls()[initial_calls..];
    assert!(new_calls.iter().any(|c| matches!(c, MockCall::Stop)));
}

#[test]
fn test_apply_equalizer_enabled() {
    let (player, mpv) = make_player();
    let bands = [3.0, -1.0, 0.0, 2.0, -2.0, 1.0, 0.5, -0.5, 4.0, -4.0];
    player.apply_equalizer(true, &bands);

    let calls = mpv.calls();
    let last_filter = calls
        .iter()
        .rev()
        .find_map(|c| match c {
            MockCall::SetAudioFilters(s) => Some(s.clone()),
            _ => None,
        })
        .expect("expected set_audio_filters to be called");
    assert!(last_filter.contains("lavfi=[equalizer="));
}

#[test]
fn test_apply_equalizer_disabled() {
    let (player, mpv) = make_player();
    let bands = [0.0; 10];
    player.apply_equalizer(false, &bands);

    let calls = mpv.calls();
    let last_filter = calls
        .iter()
        .rev()
        .find_map(|c| match c {
            MockCall::SetAudioFilters(s) => Some(s.clone()),
            _ => None,
        })
        .expect("expected set_audio_filters to be called");
    assert_eq!(last_filter, "");
}

#[test]
fn test_audio_player_new_does_not_touch_filters() {
    let (_player, mpv) = make_player();
    let calls = mpv.calls();
    assert!(!calls
        .iter()
        .any(|c| matches!(c, MockCall::SetAudioFilters(_))));
}

#[test]
fn test_build_af_string_disabled() {
    let s = build_af_string(false, &[0.0; 10]);
    assert_eq!(s, "");
}

#[test]
fn test_build_af_string_enabled() {
    let bands = [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let s = build_af_string(true, &bands);
    assert!(s.starts_with("lavfi=[equalizer="));
    assert!(s.contains("g=1.0"));
    assert!(s.contains("g=2.0"));
    assert!(s.contains("g=3.0"));
}

#[test]
fn test_handle_position_change() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);

    player.handle_position_change(42.5);
    assert!((player.position() - 42.5).abs() < 0.01);
}

#[test]
fn test_handle_duration_change_ignored_when_metadata_present() {
    // load_queue seeds duration from track.duration (180.0) — mpv's
    // own report is ignored to keep the seek bar stable on chunked
    // streams that don't have a Content-Length.
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    assert!((player.duration() - 180.0).abs() < 0.01);

    player.handle_duration_change(200.0);
    assert!((player.duration() - 180.0).abs() < 0.01);
}

#[test]
fn test_handle_duration_change_accepted_when_no_metadata() {
    // When metadata duration is 0 (rare), mpv's report fills in.
    let (player, _) = make_player();
    let mut track = make_test_track("1");
    track.duration = 0.0;
    player.load_queue(vec![track], 0);
    assert_eq!(player.duration(), 0.0);

    player.handle_duration_change(200.0);
    assert!((player.duration() - 200.0).abs() < 0.01);
}

#[test]
fn test_handle_playlist_pos_change() {
    let (player, _) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        0,
    );

    assert!(player.handle_playlist_pos_change(2));

    let state = player.state();
    assert_eq!(state.queue_index, 2);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");
    assert_eq!(player.position(), 0.0);
}

#[test]
fn test_reload_pos_change_preserves_resume_position() {
    let (player, _) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(90.0);
    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    // Failover reload arms `reloading_pos` and bakes in the transcode
    // offset base (=90).
    assert!(player.force_reload_current_track());

    // mpv's insert/play/remove dance re-enters index 0 and fires a
    // pos-change. It must report "not an advance" AND must not zero the
    // resume position/base (which would snap the seek bar to 0:00).
    assert!(!player.handle_playlist_pos_change(0));

    // The base is preserved, so a fresh 0-based transcode tick maps back
    // onto the real timeline (~95s), not ~5s.
    player.handle_position_change(5.0);
    assert!(
        (player.position() - 95.0).abs() < 0.5,
        "reload must preserve the transcode offset base, got {}",
        player.position()
    );
}

#[test]
fn test_reload_suppresses_intermediate_insert_shift() {
    // Reproduces the mobile failover glitch: mpv's insert-at pushes the
    // playing entry from idx 0 to idx 1, firing a pos-change at the shifted
    // index *before* play_index lands it back on 0. The intermediate event
    // must not be mistaken for an advance to track 2 (which cleared the
    // waveform and snapped the seek bar to 0:00).
    let (player, _) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(90.0);
    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    assert!(player.force_reload_current_track());

    // Transient insert-shift event at idx+1 (= track "2"): suppressed, and
    // it must NOT switch the current track or zero the resume base.
    assert!(!player.handle_playlist_pos_change(1));
    assert_eq!(player.state().queue_index, 0);
    assert_eq!(
        player.state().current_track.as_ref().unwrap().rating_key,
        "1"
    );

    // Landing event back on the reload index: also "not an advance".
    assert!(!player.handle_playlist_pos_change(0));
    assert_eq!(player.state().queue_index, 0);

    // Base survived both events, so a fresh 0-based transcode tick maps
    // back onto the real timeline (~95s), not ~5s.
    player.handle_position_change(5.0);
    assert!(
        (player.position() - 95.0).abs() < 0.5,
        "reload must preserve the transcode offset base through the insert \
         shift, got {}",
        player.position()
    );
}

#[test]
fn test_reload_settle_window_elapses_to_allow_advance() {
    // Once the settle window elapses without a landing event, a genuine
    // advance must be honoured again (the window is a backstop, not a
    // permanent gag).
    let (player, _) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    assert!(player.force_reload_current_track());

    // Force the window open long enough to expire.
    player.inner.lock().reload_started_at =
        Some(Instant::now() - RELOAD_SETTLE_WINDOW - Duration::from_millis(1));

    // A real advance to track "2" is now honoured.
    assert!(player.handle_playlist_pos_change(1));
    assert_eq!(player.state().queue_index, 1);
    assert_eq!(
        player.state().current_track.as_ref().unwrap().rating_key,
        "2"
    );
}

#[test]
fn test_handle_playlist_pos_change_negative_is_ignored() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);

    assert!(!player.handle_playlist_pos_change(-1));
    assert_eq!(player.state().queue_index, 0);
}

#[test]
fn test_handle_pause_change() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    player.handle_pause_change(true);
    assert_eq!(player.state().status, PlaybackStatus::Paused);

    player.handle_pause_change(false);
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_handle_idle_active() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);

    assert!(player.handle_idle_active());

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());
}

#[test]
fn test_handle_file_loaded_clears_loading() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    {
        let mut inner = player.inner.lock();
        inner.is_loading = true;
    }

    player.handle_file_loaded();

    let snapshot = player.snapshot();
    assert!(!snapshot.is_loading);
}

#[test]
fn test_derive_phase_states() {
    let now = Instant::now();
    let recent = now - Duration::from_secs(1);
    let stale = now - Duration::from_secs(STALL_THRESHOLD_SECS + 1);

    // No load yet, status=Playing → Opening (shouldn't happen in practice
    // but `derive_phase` shouldn't panic).
    assert_eq!(
        derive_phase(PlaybackStatus::Playing, None, None, now),
        Phase::Opening,
    );
    // Load just kicked off, no time-pos yet → Buffering.
    assert_eq!(
        derive_phase(PlaybackStatus::Playing, None, Some(recent), now),
        Phase::Buffering,
    );
    // Load happened, position has been arriving → Playing.
    assert_eq!(
        derive_phase(PlaybackStatus::Playing, Some(recent), Some(recent), now),
        Phase::Playing,
    );
    // Load happened, no time-pos for ages → Stalled.
    assert_eq!(
        derive_phase(PlaybackStatus::Playing, None, Some(stale), now),
        Phase::Stalled,
    );
    // Position events came in then dried up → Stalled.
    assert_eq!(
        derive_phase(PlaybackStatus::Playing, Some(stale), Some(stale), now),
        Phase::Stalled,
    );
    // Paused / Stopped passthrough.
    assert_eq!(
        derive_phase(PlaybackStatus::Paused, Some(stale), Some(stale), now),
        Phase::Paused,
    );
    assert_eq!(
        derive_phase(PlaybackStatus::Stopped, None, None, now),
        Phase::Stopped,
    );
}

/// `n` rebuffer episodes of `each`, all landing inside the window.
fn starvation_episodes(now: Instant, n: usize, each: Duration) -> Vec<(Instant, Duration)> {
    (0..n)
        .map(|i| (now - Duration::from_secs(5 * (i as u64 + 1)), each))
        .collect()
}

#[test]
fn test_source_fully_buffered_reads_cache_time_as_an_absolute_timestamp() {
    // `demuxer-cache-time` is the timestamp of the last buffered packet,
    // so a 240s track is only drained once it reads ~240 — regardless of
    // where the play head is. Subtracting the position (as this once did)
    // fires at the half-way point with a quarter of the track buffered.
    assert!(source_fully_buffered(Some(240.0), 240.0));
    assert!(source_fully_buffered(Some(239.9), 240.0));
    assert!(!source_fully_buffered(Some(120.0), 240.0));
    assert!(!source_fully_buffered(Some(180.0), 240.0));
    // Unknowns are never "drained" — callers treat that as "keep waiting",
    // which is the safe default for both the reload skip and the
    // prefetch gate.
    assert!(!source_fully_buffered(None, 240.0));
    assert!(!source_fully_buffered(Some(240.0), 0.0));
}

#[test]
fn test_is_starving_needs_a_settled_observation_window() {
    let now = Instant::now();
    // Plenty of silence, but the track only just started: opening a
    // stream costs seconds of quiet and must not read as a slow link.
    let episodes = starvation_episodes(now, 3, Duration::from_secs(8));
    assert!(!is_starving(
        &episodes,
        Duration::ZERO,
        STARVATION_MIN_OBSERVATION - Duration::from_secs(1),
        now,
    ));
    assert!(is_starving(
        &episodes,
        Duration::ZERO,
        STARVATION_MIN_OBSERVATION,
        now,
    ));
}

#[test]
fn test_is_starving_ignores_a_single_open_ended_gap() {
    let now = Instant::now();
    // One episode plus a long in-progress silence is the *dead socket*
    // shape — the stream stopped and never came back. Starvation needs
    // repeated recoveries, which prove bytes are still arriving.
    let one = starvation_episodes(now, 1, Duration::from_secs(6));
    assert!(!is_starving(
        &one,
        Duration::from_secs(40),
        Duration::from_secs(60),
        now,
    ));
    // Same silence, but delivered as two completed episodes → starving.
    let two = starvation_episodes(now, 2, Duration::from_secs(10));
    assert!(is_starving(&two, Duration::ZERO, Duration::from_secs(60), now));
}

#[test]
fn test_is_starving_requires_a_quarter_of_the_window() {
    let now = Instant::now();
    // Two brief hiccups in a full minute: annoying, not unlistenable.
    let light = starvation_episodes(now, 2, Duration::from_secs(4));
    assert!(!is_starving(
        &light,
        Duration::ZERO,
        Duration::from_secs(60),
        now,
    ));
    // The same two episodes over a shorter observed window cross the
    // ratio — 8s lost out of 30 is a link that can't keep up.
    assert!(is_starving(
        &light,
        Duration::ZERO,
        Duration::from_secs(30),
        now,
    ));
}

#[test]
fn test_is_starving_counts_the_in_progress_gap() {
    let now = Instant::now();
    let episodes = starvation_episodes(now, 2, Duration::from_secs(4));
    // Verdict must not wait for the stream to happen to resume: the
    // currently-open silence counts toward the total.
    assert!(!is_starving(
        &episodes,
        Duration::ZERO,
        Duration::from_secs(60),
        now,
    ));
    assert!(is_starving(
        &episodes,
        Duration::from_secs(8),
        Duration::from_secs(60),
        now,
    ));
    // Sub-threshold jitter between ticks isn't a rebuffer and must not
    // tip the balance.
    assert!(!is_starving(
        &episodes,
        Duration::from_secs(BUFFERING_HINT_SECS - 1),
        Duration::from_secs(60),
        now,
    ));
}

#[test]
fn test_is_starving_drops_episodes_outside_the_window() {
    let now = Instant::now();
    let stale = vec![
        (now - STARVATION_WINDOW - Duration::from_secs(5), Duration::from_secs(20)),
        (now - STARVATION_WINDOW - Duration::from_secs(1), Duration::from_secs(20)),
    ];
    assert!(!is_starving(
        &stale,
        Duration::ZERO,
        Duration::from_secs(120),
        now,
    ));
}

#[test]
fn test_position_ticks_record_rebuffer_episodes() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);

    // A tick after a long silence closes a rebuffer episode; a tick
    // right after another doesn't.
    player.handle_position_change(1.0);
    player.inner.lock().last_position_update =
        Some(Instant::now() - Duration::from_secs(BUFFERING_HINT_SECS + 2));
    player.handle_position_change(2.0);
    assert_eq!(player.inner.lock().starvation.episodes().len(), 1);
    player.handle_position_change(3.0);
    assert_eq!(player.inner.lock().starvation.episodes().len(), 1);
}

#[test]
fn test_starvation_history_does_not_cross_a_track_boundary() {
    // A track that starved must not condemn the next one, which may
    // well be playing from cache.
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(1.0);
    player.inner.lock().last_position_update =
        Some(Instant::now() - Duration::from_secs(BUFFERING_HINT_SECS + 2));
    player.handle_position_change(2.0);
    assert_eq!(player.inner.lock().starvation.episodes().len(), 1);

    assert!(player.handle_playlist_pos_change(1));
    assert!(player.inner.lock().starvation.episodes().is_empty());
}

#[test]
fn test_starvation_verdict_needs_playing_status() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    {
        let mut inner = player.inner.lock();
        let now = Instant::now();
        inner.load_started_at = Some(now - Duration::from_secs(60));
        inner.last_position_update = Some(now);
        for (t, d) in starvation_episodes(now, 3, Duration::from_secs(10)) {
            inner.starvation.record(t, d);
        }
    }
    assert!(player.is_starving());

    // A paused player isn't starving, however bad the link was.
    player.inner.lock().state.status = PlaybackStatus::Paused;
    assert!(!player.is_starving());
}

/// Put the player in a state the starvation verdict fires on, with the
/// degrade cooldown already elapsed.
fn make_starving(player: &AudioPlayer) {
    let mut inner = player.inner.lock();
    let now = Instant::now();
    inner.load_started_at = Some(now - Duration::from_secs(60));
    inner.last_position_update = Some(now);
    inner.last_degrade_at = None;
    for (t, d) in starvation_episodes(now, 3, Duration::from_secs(10)) {
        inner.starvation.record(t, d);
    }
}

fn flac_track(rk: &str) -> Track {
    Track {
        codec: Some("flac".into()),
        part_key: Some(format!("/library/parts/{rk}/file.flac")),
        duration: 300.0,
        ..make_test_track(rk)
    }
}

fn configure_for_streaming(player: &AudioPlayer, mode: PlaybackMode) {
    player.configure(
        Url::parse("http://192.168.1.100:32400").unwrap(),
        "token".into(),
        "client-id".into(),
    );
    player.update_config(PlaybackConfig {
        playback_mode: mode,
        transcode_bitrate: TranscodeBitrate::Kbps320,
        ..PlaybackConfig::default()
    });
}

#[test]
fn test_never_refuses_to_degrade_however_bad_the_link() {
    // The whole promise of the mode: a user who picked it gets buffering,
    // not a quality drop they never asked for.
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::Never);
    player.load_queue(vec![flac_track("1")], 0);
    make_starving(&player);

    assert!(player.is_starving());
    assert_eq!(player.consider_bandwidth_degrade(), None);
    assert_eq!(player.bandwidth_degrade(), None);
}

#[test]
fn test_when_slow_starts_transcoding_at_the_configured_bitrate() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);
    // Baseline is direct-play until the link is measured.
    assert!(!player.debug_snapshot().resolved_url.unwrap().contains("transcode"));

    make_starving(&player);
    let step = player.consider_bandwidth_degrade().expect("must step");
    assert_eq!(step.bitrate, TranscodeBitrate::Kbps320);
    // Plenty of track left, so it takes effect immediately.
    assert!(step.applied_to_current);
    assert!(player.debug_snapshot().resolved_url.unwrap().contains("transcode"));
}

#[test]
fn test_further_steps_walk_the_ladder_down_to_the_floor() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);

    let mut seen = Vec::new();
    for _ in 0..5 {
        make_starving(&player);
        match player.consider_bandwidth_degrade() {
            Some(step) => seen.push(step.bitrate),
            None => break,
        }
    }
    assert_eq!(
        seen,
        vec![
            TranscodeBitrate::Kbps320,
            TranscodeBitrate::Kbps256,
            TranscodeBitrate::Kbps192,
            TranscodeBitrate::Kbps128,
        ],
        "ladder must step once per rung and stop at the floor"
    );
}

#[test]
fn test_degrade_steps_are_paced() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);

    make_starving(&player);
    assert!(player.consider_bandwidth_degrade().is_some());
    // Fresh evidence inside the cooldown must not walk another rung —
    // the new stream hasn't had a full window to prove itself yet.
    make_starving(&player);
    player.inner.lock().last_degrade_at = Some(Instant::now());
    assert_eq!(player.consider_bandwidth_degrade(), None);
    assert_eq!(player.bandwidth_degrade(), Some(TranscodeBitrate::Kbps320));
}

#[test]
fn test_degrade_waits_for_the_boundary_near_the_end_of_a_track() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1"), flac_track("2")], 0);
    make_starving(&player);
    // Seconds from the end: the step is worth making, but not worth an
    // audible gap when the track is about to change anyway.
    {
        let mut inner = player.inner.lock();
        inner.position = inner.duration - 10.0;
    }
    let step = player.consider_bandwidth_degrade().expect("must step");
    assert!(!step.applied_to_current);
    // It still takes effect — the next track resolves under it.
    assert_eq!(player.bandwidth_degrade(), Some(TranscodeBitrate::Kbps320));
}

#[test]
fn test_lossy_tracks_have_nowhere_to_degrade_to() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    let mp3 = Track {
        codec: Some("mp3".into()),
        part_key: Some("/library/parts/1/file.mp3".into()),
        duration: 300.0,
        ..make_test_track("1")
    };
    player.load_queue(vec![mp3], 0);
    make_starving(&player);
    assert_eq!(player.consider_bandwidth_degrade(), None);
}

#[test]
fn test_degrade_never_applies_to_a_lossy_track_in_a_mixed_queue() {
    // The override is session-scoped, but it must not re-encode a source
    // that was never eligible in the first place.
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);
    make_starving(&player);
    assert!(player.consider_bandwidth_degrade().is_some());

    let mp3 = Track {
        codec: Some("mp3".into()),
        ..make_test_track("2")
    };
    let inner = player.inner.lock();
    assert!(!effective_stream_policy(&mp3, &inner).0);
    assert!(effective_stream_policy(&flac_track("3"), &inner).0);
}

#[test]
fn test_degrade_clears_only_on_an_external_event() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1"), flac_track("2")], 0);
    make_starving(&player);
    assert!(player.consider_bandwidth_degrade().is_some());

    // The step reloaded the current track, which arms the reload settle
    // window; let it lapse as the real 45s-of-track-left gap would.
    {
        let mut inner = player.inner.lock();
        inner.reloading_pos = None;
        inner.reload_started_at = None;
    }
    // Surviving a track change is the point — a session-scoped step
    // that reset every boundary would starve at the top of every track.
    assert!(player.handle_playlist_pos_change(1));
    assert_eq!(player.bandwidth_degrade(), Some(TranscodeBitrate::Kbps320));

    player.clear_bandwidth_degrade();
    assert_eq!(player.bandwidth_degrade(), None);
}

#[test]
fn test_restating_the_policy_clears_the_degrade() {
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);
    make_starving(&player);
    assert!(player.consider_bandwidth_degrade().is_some());

    // An unrelated settings save must leave it alone...
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::WhenSlow,
        transcode_bitrate: TranscodeBitrate::Kbps320,
        lookahead_depth: 9,
        ..PlaybackConfig::default()
    });
    assert_eq!(player.bandwidth_degrade(), Some(TranscodeBitrate::Kbps320));

    // ...but the user restating the transcode policy supersedes it.
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::WhenSlow,
        transcode_bitrate: TranscodeBitrate::Kbps192,
        ..PlaybackConfig::default()
    });
    assert_eq!(player.bandwidth_degrade(), None);
}

#[test]
fn test_locally_cached_tracks_are_never_degraded() {
    // A file playing off disk that stutters has a problem no re-encode
    // is going to fix, and the reload would be pure loss.
    let (player, _) = make_player();
    configure_for_streaming(&player, PlaybackMode::WhenSlow);
    player.load_queue(vec![flac_track("1")], 0);
    player.register_persistent_download("1".into(), PathBuf::from("/tmp/1.flac"));
    make_starving(&player);
    assert_eq!(player.consider_bandwidth_degrade(), None);
}

#[test]
fn test_load_queue_seeds_phase_timestamps() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);

    // Fresh load → Buffering (load_started_at set, no position update yet).
    let snap = player.debug_snapshot();
    assert_eq!(snap.phase, Phase::Buffering);
    assert!(snap.seconds_since_load.is_some());
    assert!(snap.seconds_since_position_update.is_none());

    // First time-pos lands → Playing.
    player.handle_position_change(0.5);
    let snap = player.debug_snapshot();
    assert_eq!(snap.phase, Phase::Playing);
    assert_eq!(snap.seconds_since_position_update, Some(0));
}

#[test]
fn test_resume_resets_progress_timer() {
    // Long pause shouldn't make resumed playback look stalled.
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(10.0);

    // Backdate the position timestamp to simulate a pause longer than
    // the stall threshold.
    {
        let mut inner = player.inner.lock();
        inner.last_position_update =
            Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 5));
    }
    player.handle_pause_change(true);
    player.handle_pause_change(false);

    assert_eq!(player.debug_snapshot().phase, Phase::Playing);
    assert!(!player.is_stalled());
}

#[test]
fn test_media_position_snapshot_detects_mid_track_stall() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    // A fresh position tick: playing, progressing, not stalled.
    player.handle_position_change(40.0);
    let threshold = Duration::from_millis(2000);
    let snap = player.media_position_snapshot(threshold);
    assert!(snap.is_playing);
    assert!(!snap.progress_stalled);
    assert!((snap.position - 40.0).abs() < 0.01);

    // Backdate the last tick past the threshold: audio has stalled, the OS
    // scrubber must be frozen at the true position (still 40s, not
    // extrapolated forward).
    player.inner.lock().last_position_update =
        Some(Instant::now() - threshold - Duration::from_millis(500));
    let snap = player.media_position_snapshot(threshold);
    assert!(snap.is_playing);
    assert!(snap.progress_stalled);
    assert!((snap.position - 40.0).abs() < 0.01);

    // Paused is never a "stall" — the pause push owns the OS surface.
    player.handle_pause_change(true);
    let snap = player.media_position_snapshot(threshold);
    assert!(!snap.is_playing);
    assert!(!snap.progress_stalled);
}

#[test]
fn test_file_ended_error_records_redacted_message() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);

    let leaky = "GET https://srv:32400/x?X-Plex-Token=SECRET failed";
    player.handle_file_ended(FileEndReason::Error(leaky.into()));

    let err = player.debug_snapshot().last_load_error.unwrap();
    assert!(!err.contains("SECRET"));
}

#[test]
fn test_handle_file_ended_error_resumes_then_holds() {
    let (player, _) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        0,
    );

    // First error: resume-at-position reload — stays on track 0.
    let out = player.handle_file_ended(FileEndReason::Error("test".into()));
    assert!(matches!(out, RecoverOutcome::Reloading(_)), "got {out:?}");
    assert_eq!(player.state().queue_index, 0);

    // Second error on the same track: hold at position (never skip or reset
    // to 0:00), so playback stays on track 0, paused, awaiting a play tap.
    let out = player.handle_file_ended(FileEndReason::Error("test".into()));
    assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
    assert_eq!(player.state().queue_index, 0);
    assert_eq!(player.state().status, PlaybackStatus::Paused);
}

#[test]
fn test_force_reload_coalesces_within_cooldown() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(30.0);
    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    // First failover reload fires.
    assert!(player.force_reload_current_track());
    // A second trigger arriving immediately (network-path flap + stall
    // watchdog + prefetch all firing for one hiccup) is coalesced by the
    // reload cooldown rather than stacked into another reload.
    assert!(
        !player.force_reload_current_track(),
        "second reload within cooldown must be suppressed"
    );
}

/// Drive a player into the held-for-recovery state: two consecutive
/// load errors on the same track exhaust the retry and hold at position.
fn hold_player_at(player: &AudioPlayer, pos: f64) {
    player.handle_position_change(pos);
    player.handle_file_ended(FileEndReason::Error("test".into()));
    let out = player.handle_file_ended(FileEndReason::Error("test".into()));
    assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
    assert_eq!(player.state().status, PlaybackStatus::Paused);
}

#[test]
fn test_recover_interrupted_playback_resumes_held() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);
    let calls_before = mpv.call_count();

    // The network came back (recovered edge / healthy watchdog verdict):
    // the hold must exit into a resume-at-position reload even though the
    // hold itself stamped the auto-reload cooldown moments ago.
    assert!(player.recover_interrupted_playback());
    assert_eq!(player.state().status, PlaybackStatus::Playing);
    assert!(!player.needs_connection_recovery());

    // The reload is the insert/play/remove dance on the held index.
    let calls = mpv.calls()[calls_before..].to_vec();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
        "expected a reload of the held entry, got {calls:?}"
    );
}

#[test]
fn test_recover_interrupted_playback_respects_user_pause() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    // A pause while held is swallowed by the status gate (status is
    // already Paused), but the *intent* must be recorded — recovery
    // must never blast audio at a user who asked for silence.
    player.pause();
    assert!(!player.needs_connection_recovery());

    let calls_before = mpv.call_count();
    assert!(!player.recover_interrupted_playback());
    assert_eq!(mpv.call_count(), calls_before, "no mpv command expected");
    assert_eq!(player.state().status, PlaybackStatus::Paused);

    // An explicit resume is the user's own retry: it re-attempts the
    // held load and clears the pause intent.
    player.resume();
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_recover_interrupted_playback_reloads_stalled_stream() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(42.0);

    // Healthy stream: nothing to recover.
    assert!(!player.needs_connection_recovery());
    assert!(!player.recover_interrupted_playback());

    // A dead socket after a silent network flip: status stays Playing
    // but position ticks stop (mpv never errors). Recovery must kick
    // the stream with a resume-at-position reload.
    player.inner.lock().last_position_update =
        Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));
    assert!(player.needs_connection_recovery());
    let calls_before = mpv.call_count();
    assert!(player.recover_interrupted_playback());
    let calls = mpv.calls()[calls_before..].to_vec();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
        "expected a reload of the stalled entry, got {calls:?}"
    );
}

#[test]
fn test_recover_interrupted_playback_coalesces_within_cooldown() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(42.0);
    player.inner.lock().last_position_update =
        Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));

    assert!(player.recover_interrupted_playback());
    // Still stalled (no position tick arrived) — a second racing trigger
    // (path event + watchdog) must coalesce, not stack reloads.
    player.inner.lock().last_position_update =
        Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));
    assert!(!player.recover_interrupted_playback());
}

#[test]
fn test_user_pause_intent_survives_status_gate_and_clears_on_play() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(10.0);

    player.pause();
    assert!(player.inner.lock().user_paused);
    player.resume();
    assert!(!player.inner.lock().user_paused);

    player.toggle_play_pause(); // Playing -> Paused
    assert!(player.inner.lock().user_paused);
    player.toggle_play_pause(); // Paused -> Playing
    assert!(!player.inner.lock().user_paused);

    // A fresh queue load supersedes any lingering pause intent.
    player.pause();
    player.load_queue(vec![make_test_track("2")], 0);
    assert!(!player.inner.lock().user_paused);
}

/// The `SetPause` values sent to mpv, in order.
fn pause_calls(mpv: &MockMpv) -> Vec<bool> {
    mpv.calls()
        .iter()
        .filter_map(|c| match c {
            MockCall::SetPause(v) => Some(*v),
            _ => None,
        })
        .collect()
}

#[test]
fn test_hold_entry_pins_mpv_paused() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    // Entering the hold must park mpv paused — with keep-open=no it
    // auto-advances past the failed entry, and an unpinned walk plays
    // whatever entry loads next, audibly, under a Paused status.
    assert_eq!(pause_calls(&mpv).last(), Some(&true));
}

#[test]
fn test_pos_change_suppressed_while_held() {
    let (player, _) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        0,
    );
    hold_player_at(&player, 30.0);

    // mpv's auto-advance walk fires pos-changes for the entries it
    // moves through. None are real advances: the hold owns the queue
    // position, and processing them used to clear the hold and cascade
    // the pointer through the whole queue.
    assert!(!player.handle_playlist_pos_change(1));
    assert!(!player.handle_playlist_pos_change(2));
    let state = player.state();
    assert_eq!(state.queue_index, 0);
    assert_eq!(
        state.current_track.as_ref().map(|t| t.rating_key.as_str()),
        Some("1")
    );
    assert!(player.inner.lock().held_for_recovery);
}

#[test]
fn test_idle_active_preserved_while_held() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    // The walk exhausting the playlist idles mpv. For a held player
    // that is expected — tearing down to Stopped would make the hold
    // unrecoverable (reloads decline on Stopped).
    assert!(!player.handle_idle_active());
    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Paused);
    assert!(state.current_track.is_some());
    assert!(player.inner.lock().held_for_recovery);

    // The preserved hold is still recoverable.
    assert!(player.recover_interrupted_playback());
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_resume_exits_hold_with_unpause() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    player.resume();
    // The exit must lift the hold's pause pin, or the reloaded track
    // sits silent under a Playing status.
    assert_eq!(pause_calls(&mpv).last(), Some(&false));
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_seek_while_held_and_user_paused_stays_paused() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);
    player.pause();

    // A paused user dragging the scrubber reloads the held track at
    // the target, but silent — their pause intent survives the exit.
    player.seek(50.0);
    assert!(!player.inner.lock().held_for_recovery);
    assert_eq!(player.state().status, PlaybackStatus::Paused);
    assert_eq!(pause_calls(&mpv).last(), Some(&true));
}

#[test]
fn test_next_out_of_hold_plays() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    player.next();
    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert!(!player.inner.lock().held_for_recovery);
    // Skip = play intent: the hold's pause pin must lift.
    assert_eq!(pause_calls(&mpv).last(), Some(&false));

    // The skip's own confirmation pos-change is processed normally
    // (the hold was released before the command).
    assert!(player.handle_playlist_pos_change(1));
}

#[test]
fn test_next_out_of_hold_respects_user_pause() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);
    player.pause();

    player.next();
    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.status, PlaybackStatus::Paused);
    // No unpause: the user asked for silence; the next track loads
    // paused under mpv's sticky pause.
    assert_eq!(pause_calls(&mpv).last(), Some(&true));
}

#[test]
fn test_previous_while_held_reloads_instead_of_dead_seek() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    hold_player_at(&player, 30.0);
    let calls_before = mpv.call_count();

    // A held track has no live mpv stream: previous()'s restart-current
    // branch must reload from the top, not issue a silent no-op seek.
    player.previous();
    let calls = mpv.calls()[calls_before..].to_vec();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
        "expected a restart reload, got {calls:?}"
    );
    assert!(!calls.iter().any(|c| matches!(c, MockCall::Seek(_))));
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_load_queue_clears_hold() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);

    // A fresh queue moots the hold; without the release, the new
    // queue's pos-change events would be suppressed as the walk.
    player.load_queue(vec![make_test_track("3")], 0);
    assert!(!player.inner.lock().held_for_recovery);
    assert!(player.handle_playlist_pos_change(0));
}

#[test]
fn test_jump_to_index_is_play_intent() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(10.0);
    player.pause();

    // Tapping a track is an explicit "play this": it supersedes the
    // pause intent and lifts mpv's sticky pause — without the explicit
    // unpause, the selected track sits silent under a Playing status.
    player.jump_to_index(1);
    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert!(!player.inner.lock().user_paused);
    assert_eq!(pause_calls(&mpv).last(), Some(&false));
}

#[test]
fn test_jump_out_of_hold_plays_even_when_user_paused() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    hold_player_at(&player, 30.0);
    player.pause();

    // Unlike next/previous (which preserve the pause), an explicit
    // track selection during an outage means "play this one now".
    player.jump_to_index(1);
    let state = player.state();
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert!(!player.inner.lock().held_for_recovery);
    assert_eq!(pause_calls(&mpv).last(), Some(&false));

    // The confirmation pos-change is processed normally.
    assert!(player.handle_playlist_pos_change(1));
}

#[test]
fn test_natural_advance_records_transition_at_final_position() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(175.0);

    // The advance mutates state to track 2 at 0:00 — the snapshot must
    // preserve track 1 at the position it actually ended.
    assert!(player.handle_playlist_pos_change(1));
    let (prev, pos, dur) = player.take_pending_transition().unwrap();
    assert_eq!(prev.rating_key, "1");
    assert_eq!(pos, 175.0);
    assert_eq!(dur, 180.0);

    // Consumed — a second take yields nothing.
    assert!(player.take_pending_transition().is_none());
}

#[test]
fn test_skip_confirmation_keeps_preskip_position() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(42.0);

    // The skip records the transition synchronously; mpv's confirmation
    // pos-change (same rating key, position already zeroed) must not
    // clobber the snapshot with 0:00.
    player.next();
    assert!(player.handle_playlist_pos_change(1));
    let (prev, pos, _) = player.take_pending_transition().unwrap();
    assert_eq!(prev.rating_key, "1");
    assert_eq!(pos, 42.0);
}

#[test]
fn test_previous_records_transition() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    assert!(player.handle_playlist_pos_change(1));
    let _ = player.take_pending_transition();
    player.handle_position_change(2.0);

    // Under the 3s restart threshold, previous() goes back a track.
    player.previous();
    let (prev, pos, _) = player.take_pending_transition().unwrap();
    assert_eq!(prev.rating_key, "2");
    assert_eq!(pos, 2.0);
}

#[test]
fn test_jump_records_transition() {
    let (player, _) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
        0,
    );
    player.handle_position_change(60.0);

    player.jump_to_index(2);
    let (prev, pos, _) = player.take_pending_transition().unwrap();
    assert_eq!(prev.rating_key, "1");
    assert_eq!(pos, 60.0);
}

#[test]
fn test_skip_off_queue_end_records_transition() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(171.0);

    // No pos-change fires on this path (mpv just goes idle) — the idle
    // callback consumes the snapshot to close the track at 95%, where
    // it deserves its scrobble.
    player.next();
    assert_eq!(player.state().status, PlaybackStatus::Stopped);
    let (prev, pos, _) = player.take_pending_transition().unwrap();
    assert_eq!(prev.rating_key, "1");
    assert_eq!(pos, 171.0);
}

#[test]
fn test_load_queue_and_stop_clear_pending_transition() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    player.handle_position_change(42.0);
    player.next();

    // A fresh queue's close-out is reported by the caller with the
    // player still in its pre-load state — a leftover snapshot must not
    // fire on the new queue's first pos-change.
    player.load_queue(vec![make_test_track("3")], 0);
    assert!(player.take_pending_transition().is_none());

    // stop() likewise discards any unconsumed snapshot outright.
    player.handle_position_change(42.0);
    player.next();
    player.stop();
    assert!(player.take_pending_transition().is_none());
}

#[test]
fn test_queue_reload_pos_change_records_nothing() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);

    // The Replace load's pos-change re-confirms the track load_queue
    // already installed; play_tracks reported that start itself.
    assert!(player.handle_playlist_pos_change(0));
    assert!(player.take_pending_transition().is_none());
}

#[test]
fn test_load_queue_resolves_direct_play_urls() {
    let (player, mpv) = make_player();
    let track = make_test_track("123");
    player.load_queue(vec![track], 0);

    let calls = mpv.calls();
    let load = calls
        .iter()
        .find(|c| matches!(c, MockCall::LoadFile { .. }));
    assert!(load.is_some());
    if let MockCall::LoadFile { url, .. } = load.unwrap() {
        assert!(url.contains("test-token"));
        assert!(url.contains("/library/parts/123/file.flac"));
    }
}

#[test]
fn test_cached_track_uses_file_url() {
    let (player, mpv) = make_player();

    player.with_cache(|cache| {
        cache.insert(
            "123".into(),
            PathBuf::from("/tmp/cache/123.flac"),
            1000,
        );
    });

    let track = make_test_track("123");
    player.load_queue(vec![track], 0);

    let calls = mpv.calls();
    let load = calls
        .iter()
        .find(|c| matches!(c, MockCall::LoadFile { .. }));
    if let Some(MockCall::LoadFile { url, .. }) = load {
        assert!(url.starts_with("file://"));
        assert!(url.contains("/tmp/cache/123.flac"));
    }
}

#[test]
fn test_persistent_download_wins_over_lru_cache() {
    let (player, mpv) = make_player();

    // LRU says /tmp/cache, persistent says /tmp/downloads — persistent wins.
    player.with_cache(|cache| {
        cache.insert(
            "123".into(),
            PathBuf::from("/tmp/cache/123.flac"),
            1000,
        );
    });
    player.register_persistent_download(
        "123".into(),
        PathBuf::from("/tmp/downloads/123.flac"),
    );

    let track = make_test_track("123");
    player.load_queue(vec![track], 0);

    let calls = mpv.calls();
    let load = calls
        .iter()
        .find(|c| matches!(c, MockCall::LoadFile { .. }));
    match load {
        Some(MockCall::LoadFile { url, .. }) => {
            assert!(url.starts_with("file://"));
            assert!(
                url.contains("/tmp/downloads/123.flac"),
                "persistent download should win, got {url}"
            );
        }
        _ => panic!("expected LoadFile call"),
    }
}

#[test]
fn test_unregister_persistent_download() {
    let (player, _) = make_player();
    player.register_persistent_download(
        "123".into(),
        PathBuf::from("/tmp/downloads/123.flac"),
    );
    assert!(player.has_persistent_download("123"));
    player.unregister_persistent_download("123");
    assert!(!player.has_persistent_download("123"));
}

#[test]
fn test_rehydrate_persistent_cache_replaces_contents() {
    let (player, _) = make_player();
    player.register_persistent_download(
        "old".into(),
        PathBuf::from("/tmp/downloads/old.flac"),
    );

    let mut entries = HashMap::new();
    entries.insert("new".into(), PathBuf::from("/tmp/downloads/new.flac"));
    player.rehydrate_persistent_cache(entries);

    assert!(!player.has_persistent_download("old"));
    assert!(player.has_persistent_download("new"));
}

#[test]
fn test_snapshot_reflects_state() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(30.0);
    player.handle_duration_change(180.0);
    player.set_volume(80.0);

    let snapshot = player.snapshot();
    assert_eq!(snapshot.state.status, PlaybackStatus::Playing);
    assert!((snapshot.position - 30.0).abs() < 0.1);
    assert!((snapshot.duration - 180.0).abs() < 0.1);
    assert!((snapshot.volume - 80.0).abs() < 0.1);
}

#[test]
fn test_load_queue_generates_new_session_id() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    let session1 = player.play_session_id();

    player.load_queue(vec![make_test_track("2")], 0);
    let session2 = player.play_session_id();

    assert_ne!(session1, session2);
}

#[test]
fn test_rewrite_stale_playlist_urls_replaces_non_cached() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![
            make_test_track("1"),
            make_test_track("2"),
            make_test_track("3"),
        ],
        0,
    );

    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    player.rewrite_stale_playlist_urls();

    let calls = mpv.calls();
    let removes: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
        .collect();
    let inserts: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::LoadFileAt { .. }))
        .collect();

    // Tracks 1 and 2 (indices 1, 2) should be rewritten; track 0 (current) skipped
    assert_eq!(removes.len(), 2);
    assert_eq!(inserts.len(), 2);

    // Verify new URLs contain the new server
    for call in &inserts {
        if let MockCall::LoadFileAt { url, .. } = call {
            assert!(url.contains("new.server:32400"));
            assert!(url.contains("new-token"));
        }
    }
}

#[test]
fn test_rewrite_skips_cached_and_current() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![
            make_test_track("1"),
            make_test_track("2"),
            make_test_track("3"),
        ],
        0,
    );

    // Cache track "2" in LRU
    player.with_cache(|cache| {
        cache.insert("2".into(), PathBuf::from("/tmp/cached_2.flac"), 1000);
    });

    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    player.rewrite_stale_playlist_urls();

    let calls = mpv.calls();
    let removes: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
        .collect();

    // Only track "3" (index 2) should be rewritten; "1" is current, "2" is cached
    assert_eq!(removes.len(), 1);
    if let MockCall::PlaylistRemove(idx) = removes[0] {
        assert_eq!(*idx, 2);
    }
}

#[test]
fn test_rewrite_skips_persistent_downloads() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![
            make_test_track("1"),
            make_test_track("2"),
            make_test_track("3"),
        ],
        0,
    );

    player.register_persistent_download("2".into(), PathBuf::from("/downloads/2.flac"));

    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    player.rewrite_stale_playlist_urls();

    let calls = mpv.calls();
    let removes: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
        .collect();

    // Only track "3" (index 2) rewritten; "1" is current, "2" has persistent download
    assert_eq!(removes.len(), 1);
}

#[test]
fn test_lookahead_warm_targets_only_includes_cached_audio() {
    let (player, _mpv) = make_player();
    let mut t1 = make_test_track("1");
    t1.thumb = Some("/art/1".into());
    let mut t2 = make_test_track("2");
    t2.thumb = Some("/art/2".into());
    let t3 = make_test_track("3"); // thumb stays None
    player.load_queue(vec![t1, t2, t3], 0);

    // "2" cached in the LRU, "3" as a permanent download. "1" (the
    // current track) has no cached audio.
    player.with_cache(|cache| {
        cache.insert("2".into(), PathBuf::from("/tmp/2.flac"), 1000);
    });
    player.register_persistent_download("3".into(), PathBuf::from("/downloads/3.flac"));

    // include_current = true, yet "1" is excluded because its audio
    // isn't secured — we never warm extras for an unplayable track.
    let targets = player.lookahead_warm_targets(true);
    let keys: Vec<&str> = targets.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, vec!["2", "3"]);

    let warm2 = targets.iter().find(|t| t.rating_key == "2").unwrap();
    assert_eq!(warm2.thumb.as_deref(), Some("/art/2"));
    assert_eq!(warm2.audio_path, PathBuf::from("/tmp/2.flac"));

    let warm3 = targets.iter().find(|t| t.rating_key == "3").unwrap();
    assert_eq!(warm3.thumb, None);
    assert_eq!(warm3.audio_path, PathBuf::from("/downloads/3.flac"));
}

#[test]
fn test_force_reload_current_replaces_active_entry() {
    let (player, mpv) = make_player();
    player.load_queue(
        vec![make_test_track("1"), make_test_track("2")],
        0,
    );
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    let reloaded = player.force_reload_current_track();
    assert!(reloaded);

    let calls = mpv.calls();
    // Insert fresh URL at idx 0, play it, then remove the stale (now at idx 1).
    assert!(calls
        .iter()
        .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, url, .. } if url.contains("new.server:32400") && url.contains("new-token"))));
    assert!(calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(0))));
    assert!(calls
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistRemove(1))));
}

#[test]
fn test_force_reload_current_skips_cached() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.with_cache(|cache| {
        cache.insert("1".into(), PathBuf::from("/tmp/cached_1.flac"), 1000);
    });
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    let reloaded = player.force_reload_current_track();

    assert!(!reloaded);
    assert!(mpv.calls().is_empty());
}

#[test]
fn test_force_reload_current_skips_persistent_download() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1")], 0);
    player.register_persistent_download("1".into(), PathBuf::from("/downloads/1.flac"));
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    let reloaded = player.force_reload_current_track();

    assert!(!reloaded);
    assert!(mpv.calls().is_empty());
}

#[test]
fn test_force_reload_current_skips_when_stopped() {
    let (player, mpv) = make_player();
    // No load_queue — status stays Stopped.
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    let reloaded = player.force_reload_current_track();

    assert!(!reloaded);
    assert!(mpv.calls().is_empty());
}

#[test]
fn test_force_reload_resumes_direct_play_with_start_option() {
    let (player, mpv) = make_player();
    player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
    // ~60s of playback elapsed before the connection changed.
    player.handle_position_change(60.0);
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    assert!(player.force_reload_current_track());

    // Direct-play (default mode is Never) resumes via an mpv `start=`
    // seek: the URL stays a plain part URL, the option carries the seek.
    let (url, options) = mpv
        .calls()
        .into_iter()
        .find_map(|c| match c {
            MockCall::LoadFileAt {
                index: 0,
                url,
                options,
            } => Some((url, options)),
            _ => None,
        })
        .expect("expected a LoadFileAt at index 0");
    assert!(
        !url.contains("offset="),
        "direct-play must not use a transcode offset"
    );
    let opts = options.expect("expected a start= option for the resume");
    assert!(opts.contains("start=60"), "expected start=60.x, got {opts}");
}

#[test]
fn test_force_reload_resumes_transcode_with_offset() {
    let (player, mpv) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(75.0);
    mpv.calls.lock().clear();

    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    assert!(player.force_reload_current_track());

    let (url, options) = mpv
        .calls()
        .into_iter()
        .find_map(|c| match c {
            MockCall::LoadFileAt {
                index: 0,
                url,
                options,
            } => Some((url, options)),
            _ => None,
        })
        .expect("expected a LoadFileAt at index 0");
    // Transcode resume bakes the offset into the URL (server-side seek)
    // with its companion params — and carries no mpv start= option.
    assert!(url.contains("offset=75"), "expected offset=75 in url, got {url}");
    assert!(
        url.contains("mediaBufferSize=1024"),
        "expected offset companions, got {url}"
    );
    assert!(
        options.is_none(),
        "transcode resume must not use start=, got {options:?}"
    );

    // `position_base` now remaps mpv's 0-based stream: a fresh time-pos
    // of 5s reads as 80s on the track timeline.
    player.handle_position_change(5.0);
    assert!(
        (player.snapshot().position - 80.0).abs() < 0.01,
        "position should map through the offset base, got {}",
        player.snapshot().position
    );
}

#[test]
fn test_transcode_resume_failure_holds_at_position() {
    let (player, mpv) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.load_queue(vec![make_test_track("1")], 0);
    player.handle_position_change(90.0);
    player.update_server_connection(
        Url::parse("http://new.server:32400").unwrap(),
        "new-token".into(),
        true,
    );
    assert!(player.force_reload_current_track()); // offset resume, base = 90
    mpv.calls.lock().clear();

    // The offset transcode start is refused (e.g. HTTP 400) → mpv errors.
    // Recovery must NOT reset to 0:00 or skip — it holds the track at its
    // position so a play tap can re-attempt. (The immediate retry lands
    // inside the reload cooldown, so it holds rather than thrashing.)
    let out = player.handle_file_ended(FileEndReason::Error("HTTP 400 Bad Request".into()));
    assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
    assert_eq!(player.state().queue_index, 0, "must not skip the track");
    assert_eq!(player.state().status, PlaybackStatus::Paused);
    assert!(
        (player.snapshot().position - 90.0).abs() < 0.5,
        "held position must stay at ~90s, not reset to 0:00"
    );

    // A play tap re-attempts a resume-at-position (offset) load, never a
    // restart from the top.
    mpv.calls.lock().clear();
    player.resume();
    let url = mpv
        .calls()
        .into_iter()
        .find_map(|c| match c {
            MockCall::LoadFileAt { index: 0, url, .. } => Some(url),
            _ => None,
        })
        .expect("expected a re-attempt LoadFileAt at index 0");
    assert!(
        url.contains("offset="),
        "re-attempt must resume at position, got {url}"
    );
}

// --- Restored-queue materialisation -------------------------------------
//
// A queue restored from disk is real state (it drives the UI) with an empty
// mpv playlist behind it. That split is the one invariant these tests exist
// to pin: nothing may command mpv about the queue until a genuine play
// intent materialises it, and every transport path must materialise onto the
// track the user actually asked for.

/// Restore a three-track queue sitting 90s into track "2".
fn restored_player() -> (AudioPlayer, Arc<MockMpv>) {
    let (player, mpv) = make_player();
    let tracks = vec![
        make_test_track("1"),
        make_test_track("2"),
        make_test_track("3"),
    ];
    player.restore_queue(tracks, 1, 90.0);
    (player, mpv)
}

#[test]
fn test_restore_queue_issues_no_mpv_commands() {
    let (player, mpv) = restored_player();

    assert_eq!(
        mpv.call_count(),
        0,
        "restoring must not touch mpv — a launch costs no network"
    );

    let state = player.state();
    assert_eq!(state.queue.len(), 3);
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");
    // Paused, not Stopped: the UI should read "paused at 1:30", and
    // `user_paused` keeps every automatic recovery path away from it.
    assert_eq!(state.status, PlaybackStatus::Paused);
    assert!((player.position() - 90.0).abs() < 0.01);
    assert_eq!(player.duration(), 180.0);
}

#[test]
fn test_restore_queue_declines_when_a_queue_is_already_loaded() {
    let (player, _) = make_player();
    player.load_queue(vec![make_test_track("live")], 0);

    player.restore_queue(vec![make_test_track("stale")], 0, 30.0);

    assert_eq!(
        player.state().current_track.as_ref().unwrap().rating_key,
        "live",
        "a restore must never displace live playback"
    );
}

#[test]
fn test_restore_queue_clamps_a_position_past_the_end() {
    let (player, _) = make_player();
    player.restore_queue(vec![make_test_track("1")], 0, 9_999.0);
    assert!(
        player.position() <= 180.0,
        "restored position must clamp to the track, got {}",
        player.position()
    );
}

#[test]
fn test_restored_queue_is_not_a_recovery_candidate() {
    let (player, mpv) = restored_player();

    // `user_paused` is what buys this: the stall watchdog, the
    // connection-recovered edge and foreground resync all funnel through
    // these two, and none may start audio nobody asked for.
    assert!(!player.needs_connection_recovery());
    assert!(!player.recover_interrupted_playback());
    assert_eq!(mpv.call_count(), 0);
}

#[test]
fn test_first_play_materialises_at_the_restored_position() {
    let (player, mpv) = restored_player();

    player.resume();

    // Whole queue pushed into mpv: Replace for the first entry, Append for
    // the rest.
    let loads: Vec<_> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { url, mode, options } => Some((url, mode, options)),
            _ => None,
        })
        .collect();
    assert_eq!(loads.len(), 3, "the whole queue must be loaded");
    assert!(matches!(loads[0].1, LoadMode::Replace));
    assert!(matches!(loads[1].1, LoadMode::Append));

    // Direct-play resumes via an mpv `start=` on the playing entry only.
    assert_eq!(
        loads[1].2.as_deref(),
        Some("start=90.000"),
        "the restored entry must resume at its saved position"
    );
    assert!(
        loads[0].2.as_deref().is_none_or(|o| !o.contains("start=")),
        "the resume must not leak onto other entries"
    );
    assert!(
        loads[2].2.as_deref().is_none_or(|o| !o.contains("start=")),
        "the resume must not leak onto other entries"
    );

    // ...and playback actually starts on the restored track.
    assert!(mpv
        .calls()
        .iter()
        .any(|c| matches!(c, MockCall::PlaylistPlayIndex(1))));
    assert!(mpv
        .calls()
        .iter()
        .any(|c| matches!(c, MockCall::SetPause(false))));

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.queue_index, 1);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");
}

#[test]
fn test_materialising_a_transcode_resume_sets_the_position_base() {
    let (player, mpv) = make_player();
    player.update_config(PlaybackConfig {
        playback_mode: PlaybackMode::Always,
        ..PlaybackConfig::default()
    });
    player.restore_queue(vec![make_test_track("1"), make_test_track("2")], 0, 90.0);

    player.resume();

    // A transcode stream is `Accept-Ranges: none`, so the resume is baked
    // into the URL server-side rather than sought by mpv.
    let url = mpv
        .calls()
        .into_iter()
        .find_map(|c| match c {
            MockCall::LoadFile { url, mode: LoadMode::Replace, .. } => Some(url),
            _ => None,
        })
        .expect("expected a Replace load for the restored entry");
    assert!(
        url.contains("offset=90"),
        "transcode resume must carry a server-side offset, got {url}"
    );

    // mpv sees a fresh 0-based stream, so `position_base` remaps its ticks
    // back onto the track timeline.
    player.handle_position_change(5.0);
    assert!(
        (player.position() - 95.0).abs() < 0.5,
        "expected ~95s on the track timeline, got {}",
        player.position()
    );
}

#[test]
fn test_materialising_a_resume_carries_no_stream_record() {
    let (player, mpv) = make_player();
    player.set_stream_record_dir(PathBuf::from("/tmp/ramus-test-record"));
    player.restore_queue(vec![make_test_track("1"), make_test_track("2")], 0, 90.0);

    player.resume();

    let loads: Vec<_> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { mode, options, .. } => Some((mode, options)),
            _ => None,
        })
        .collect();
    // The resumed entry would record from partway through the source, and
    // the analyser would render a spectrum offset from the audio.
    assert!(
        !loads[0].1.as_deref().unwrap_or("").contains("stream-record"),
        "a resumed entry must not be captured, got {:?}",
        loads[0].1
    );
    // Entries loading from the top still capture normally.
    assert!(
        loads[1].1.as_deref().unwrap_or("").contains("stream-record"),
        "a from-the-top entry must still capture, got {:?}",
        loads[1].1
    );
}

#[test]
fn test_next_on_a_restored_queue_materialises_at_the_next_track() {
    let (player, mpv) = restored_player();

    player.next();

    assert_eq!(player.state().queue_index, 2);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "3");
    // Nothing may resume: the skipped-past track must not get a `start=`,
    // and no stream should have been opened for it.
    let options: Vec<_> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { options, .. } => options,
            _ => None,
        })
        .collect();
    assert!(
        options.iter().all(|o| !o.contains("start=")),
        "a skip must start the new track from the top, got {options:?}"
    );
}

#[test]
fn test_next_past_the_end_of_a_restored_queue_stops() {
    let (player, mpv) = make_player();
    player.restore_queue(vec![make_test_track("1")], 0, 30.0);

    player.next();

    // The materialise target is out of range, so it declines and the normal
    // stop-at-end branch runs.
    assert_eq!(player.state().status, PlaybackStatus::Stopped);
    assert!(player.state().current_track.is_none());
    assert!(mpv.calls().iter().any(|c| matches!(c, MockCall::Stop)));
}

#[test]
fn test_previous_on_a_restored_queue_respects_the_restart_threshold() {
    // 90s in — past the threshold, so `previous` restarts the current track.
    let (player, mpv) = restored_player();
    player.previous();
    assert_eq!(player.state().queue_index, 1);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "2");
    let options: Vec<_> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { options, .. } => options,
            _ => None,
        })
        .collect();
    assert!(
        options.iter().all(|o| !o.contains("start=")),
        "a restart must begin at 0:00, got {options:?}"
    );

    // Under the threshold it steps back a track instead.
    let (player, _) = make_player();
    player.restore_queue(
        vec![make_test_track("1"), make_test_track("2")],
        1,
        1.0,
    );
    player.previous();
    assert_eq!(player.state().queue_index, 0);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "1");
}

#[test]
fn test_jump_to_index_on_a_restored_queue_materialises_there() {
    let (player, mpv) = restored_player();

    player.jump_to_index(2);

    assert_eq!(player.state().queue_index, 2);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "3");
    assert_eq!(player.state().status, PlaybackStatus::Playing);
    assert!(mpv
        .calls()
        .iter()
        .any(|c| matches!(c, MockCall::LoadFile { mode: LoadMode::Replace, .. })));
}

#[test]
fn test_seek_on_a_restored_queue_materialises_at_the_target() {
    let (player, mpv) = restored_player();

    player.seek(120.0);

    let loads: Vec<_> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { options, .. } => options,
            _ => None,
        })
        .collect();
    assert!(
        loads.iter().any(|o| o.contains("start=120.000")),
        "scrubbing a restored track must load it at the drag target, got {loads:?}"
    );
    // A bare mpv seek against an idle player would have been silent.
    assert!(
        !mpv.calls().iter().any(|c| matches!(c, MockCall::Seek(_))),
        "must load rather than seek an unmaterialised queue"
    );
}

#[test]
fn test_materialising_clears_the_pending_flag() {
    let (player, mpv) = restored_player();

    player.resume();
    let after_first = mpv.call_count();
    assert!(after_first > 0);

    // A second play command is an ordinary unpause against a live playlist,
    // never another full queue load.
    mpv.calls.lock().clear();
    player.pause();
    player.resume();
    assert!(
        !mpv.calls()
            .iter()
            .any(|c| matches!(c, MockCall::LoadFile { .. })),
        "the queue must only materialise once, got {:?}",
        mpv.calls()
    );
}

#[test]
fn test_appending_to_a_restored_queue_touches_state_only() {
    let (player, mpv) = restored_player();

    player.append_to_queue(vec![make_test_track("4")]);

    assert_eq!(player.state().queue.len(), 4);
    assert_eq!(player.state().queue[3].rating_key, "4");
    // Loading now would place the entry at mpv index 0 while we hold it at
    // index 3 — a permanent desync between the two playlists.
    assert_eq!(
        mpv.call_count(),
        0,
        "appending must not load into an empty mpv playlist"
    );

    // The appended track is part of the queue that materialises later.
    player.resume();
    let loads = mpv
        .calls()
        .into_iter()
        .filter(|c| matches!(c, MockCall::LoadFile { .. }))
        .count();
    assert_eq!(loads, 4);
}

#[test]
fn test_inserting_into_a_restored_queue_touches_state_only() {
    let (player, mpv) = restored_player();

    player.insert_next(vec![make_test_track("new")]);

    assert_eq!(player.state().queue.len(), 4);
    assert_eq!(player.state().queue[2].rating_key, "new");
    assert_eq!(mpv.call_count(), 0);
}

#[test]
fn test_removing_from_a_restored_queue_touches_state_only() {
    let (player, mpv) = restored_player();

    player.remove_from_queue(0);

    assert_eq!(player.state().queue.len(), 2);
    // The playing track followed the shift.
    assert_eq!(player.state().queue_index, 0);
    assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "2");
    assert!(
        !mpv.calls()
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistRemove(_))),
        "there is no mpv playlist to remove from yet"
    );
}

#[test]
fn test_stop_clears_the_pending_materialise() {
    let (player, mpv) = restored_player();

    player.stop();
    assert!(player.state().queue.is_empty());

    // Nothing left to materialise: a stray transport command must not try to
    // load a queue that no longer exists.
    mpv.calls.lock().clear();
    player.resume();
    assert!(
        !mpv.calls()
            .iter()
            .any(|c| matches!(c, MockCall::LoadFile { .. })),
        "a cleared queue must not materialise"
    );
}

#[test]
fn test_load_queue_at_applies_the_resume_to_the_start_index_only() {
    let (player, mpv) = make_player();

    player.load_queue_at(
        vec![
            make_test_track("1"),
            make_test_track("2"),
            make_test_track("3"),
        ],
        2,
        Some(45.0),
    );

    let options: Vec<Option<String>> = mpv
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            MockCall::LoadFile { options, .. } => Some(options),
            _ => None,
        })
        .collect();
    assert_eq!(options.len(), 3);
    assert_eq!(options[2].as_deref(), Some("start=45.000"));
    assert!(options[0].as_deref().is_none_or(|o| !o.contains("start=")));
    assert!(options[1].as_deref().is_none_or(|o| !o.contains("start=")));
    assert!((player.position() - 45.0).abs() < 0.01);
}

#[test]
fn test_materialising_latches_for_session_reporting() {
    let (player, _) = restored_player();

    // Nothing has materialised yet.
    assert!(!player.take_just_materialized());

    player.resume();
    assert!(
        player.take_just_materialized(),
        "the platform layer needs this to open the Plex session — a fresh \
         queue load clears the transition snapshot the pos-change callback \
         would otherwise report from"
    );
    // Consumed, so a second command can't re-report the same start.
    assert!(!player.take_just_materialized());
}

#[test]
fn test_a_declined_materialise_does_not_latch() {
    let (player, _) = make_player();
    player.restore_queue(vec![make_test_track("1")], 0, 30.0);

    // Out of range: declines, so nothing started and nothing may be reported.
    player.jump_to_index(99);
    assert!(!player.take_just_materialized());
    assert_eq!(player.state().status, PlaybackStatus::Paused);
}

#[test]
fn test_mpv_going_idle_does_not_tear_down_a_restored_queue() {
    // mpv runs with `idle=yes` and reports idle-active the moment it
    // initialises at app start, which races the restore. Processing that as
    // a queue completion wiped `current_track` and flipped the status to
    // Stopped while leaving the queue populated — so the UI showed no player
    // at all, and every transport path read Stopped and declined. Observed
    // on device with a 1141-track restore.
    let (player, _) = restored_player();

    assert!(
        !player.handle_idle_active(),
        "an idle mpv is only a queue completion once mpv has been given the queue"
    );

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Paused);
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");
    assert_eq!(state.queue.len(), 3);
    assert!((player.position() - 90.0).abs() < 0.01);

    // ...and the queue is still playable afterwards.
    player.resume();
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}

#[test]
fn test_mpv_going_idle_still_tears_down_a_materialised_queue() {
    // The guard above must not swallow a real queue completion.
    let (player, _) = restored_player();
    player.resume();

    assert!(player.handle_idle_active());
    assert_eq!(player.state().status, PlaybackStatus::Stopped);
    assert!(player.state().current_track.is_none());
}

#[test]
fn test_an_ordinary_queue_load_does_not_latch() {
    let (player, _) = make_player();

    // play_tracks reports its own start; a false latch here would double it.
    player.load_queue(vec![make_test_track("1")], 0);
    assert!(!player.take_just_materialized());
}

#[test]
fn test_mpv_startup_events_do_not_disturb_a_restored_queue() {
    // mpv initialises with `idle=yes` and `pause=no` and announces both the
    // moment it starts, which races the restore. Every one of these events is
    // about a player that has never been given our tracks, so none may be
    // applied. Observed on device: the `pause=false` report flipped the
    // restored Paused status to Playing, so the UI showed a playing track
    // with a frozen seek bar and no audio.
    let (player, mpv) = restored_player();

    player.handle_pause_change(false);
    player.handle_position_change(0.0);
    let _ = player.handle_playlist_pos_change(0);
    assert!(!player.handle_idle_active());

    let state = player.state();
    assert_eq!(
        state.status,
        PlaybackStatus::Paused,
        "mpv's startup pause report must not claim the restored queue is playing"
    );
    assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");
    assert_eq!(state.queue_index, 1);
    assert!(
        (player.position() - 90.0).abs() < 0.01,
        "restored position must survive mpv's startup chatter, got {}",
        player.position()
    );
    assert_eq!(mpv.call_count(), 0);

    // Still materialises correctly afterwards.
    player.resume();
    assert_eq!(player.state().status, PlaybackStatus::Playing);
    assert_eq!(player.state().queue_index, 1);
}

#[test]
fn test_pause_reports_are_honoured_once_materialised() {
    // The guard must not outlive the restore: mpv's pause property is the
    // authority again the moment it owns the queue.
    let (player, _) = restored_player();
    player.resume();
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    player.handle_pause_change(true);
    assert_eq!(player.state().status, PlaybackStatus::Paused);
    player.handle_pause_change(false);
    assert_eq!(player.state().status, PlaybackStatus::Playing);
}
