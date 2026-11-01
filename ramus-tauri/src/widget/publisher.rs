//! Publishes the playback snapshot the home-screen widget renders from.
//! Called from every place that already tells the OS media controls about
//! a state change, plus queue persistence and sign-out; the snapshot is
//! only rewritten when something the widget shows has changed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use ramus_core::models::{PlaybackStatus, Track};
use ramus_core::playback::waveform::downsample_levels;
use ramus_core::widget::{
    build_snapshot, needs_publish, widget_colors, SnapshotInput, WidgetTrack, WAVEFORM_POINTS,
};
use tauri::{AppHandle, Manager};

use super::files::{self, ArtStep};
use crate::commands::{library, playback};
use crate::state::AppState;

/// Bursts of state changes (a skip changes track and state together)
/// within this window are written once.
const COALESCE: Duration = Duration::from_millis(250);

/// Longest a publish waits on the cover or the waveform, so a slow or dead
/// connection can't hold back the state the widget shows.
const FETCH_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Default)]
struct Inner {
    app: OnceLock<AppHandle>,
    /// The widget folder, resolved once from the plugin.
    dir: OnceLock<Option<PathBuf>>,
    pending: AtomicBool,
    /// Serialises publishes so two can't interleave their reads and writes.
    publishing: tokio::sync::Mutex<()>,
    /// Thumb of the cover last copied in, so a pause doesn't recopy it.
    art_thumb: Mutex<Option<String>>,
    /// The last track's waveform, so state changes on one track don't
    /// refetch it.
    waveform: Mutex<Option<(String, Option<Vec<f32>>)>>,
}

#[derive(Clone, Default)]
pub struct WidgetPublisher {
    inner: Arc<Inner>,
}

/// The widget's view of a track. Holds no URLs: covers travel as files.
pub fn widget_track(track: &Track, duration_sec: f64) -> WidgetTrack {
    WidgetTrack {
        rating_key: track.rating_key.clone(),
        title: track.title.clone(),
        artist: track.display_artist().to_string(),
        album: track.album_title.clone(),
        duration_sec,
    }
}

impl WidgetPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start publishing. Until this is called (at the end of `setup()`),
    /// notifications are ignored.
    pub fn attach(&self, app: AppHandle) {
        let _ = self.inner.app.set(app);
    }

    /// Publish soon, coalescing with other notifications in the next 250 ms.
    pub fn notify(&self) {
        if !cfg!(target_os = "ios") || self.inner.app.get().is_none() {
            return;
        }
        if self.inner.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            inner.pending.store(false, Ordering::Release);
            publish(&inner, false).await;
        });
    }

    /// Publish now, without coalescing.
    pub fn publish_now(&self) {
        if !cfg!(target_os = "ios") || self.inner.app.get().is_none() {
            return;
        }
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move { publish(&inner, false).await });
    }

    /// Write a fresh snapshot even if it matches the last one, and return
    /// once it is written. Used after a widget command, so the widget
    /// re-plans from the app's real state even when the command found
    /// nothing to do.
    pub async fn publish_forced(&self) {
        if !cfg!(target_os = "ios") || self.inner.app.get().is_none() {
            return;
        }
        publish(&self.inner, true).await;
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn signed_in() -> bool {
    ramus_core::plex::token_store::TokenStore::new()
        .ok()
        .and_then(|ts| ramus_core::plex::auth::stored_server_config(&ts))
        .is_some()
}

async fn publish(inner: &Inner, force: bool) {
    let Some(app) = inner.app.get() else { return };
    let Some(dir) = inner.dir.get_or_init(|| container_dir(app)).clone() else {
        return;
    };
    let _serial = inner.publishing.lock().await;
    let state = app.state::<AppState>();

    // What is playing: the cast's view while casting, else the local player.
    let (track, status, position, has_queue, cast_player) = if state.cast.is_active() {
        let view = state.cast.view();
        let track = view.index.and_then(|i| view.tracks.get(i)).cloned();
        let status = match view.status.as_str() {
            "playing" => PlaybackStatus::Playing,
            "paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        };
        let duration = view.duration;
        (
            track.map(|t| (widget_track(&t, duration), t)),
            status,
            view.position,
            !view.tracks.is_empty(),
            view.player.map(|p| p.name),
        )
    } else {
        let ps = state.player.state();
        let position = state.player.position();
        let has_queue = !ps.queue.is_empty();
        (
            ps.current_track.map(|t| (widget_track(&t, t.duration), t)),
            ps.status,
            position,
            has_queue,
            None,
        )
    };

    let (colors, waveform) = match &track {
        Some((_, t)) => (
            album_colors(&state, t).await,
            waveform_for(inner, &state, t).await,
        ),
        None => (None, None),
    };

    let prev = files::read_snapshot(&dir);
    let snap = build_snapshot(
        SnapshotInput {
            signed_in: signed_in(),
            status,
            track: track.as_ref().map(|(w, _)| w.clone()),
            position_sec: position,
            has_queue,
            resume_enabled: state.settings.read().resume_queue_on_launch,
            colors,
            waveform,
            cast_player,
            now: now_secs(),
        },
        prev.as_ref(),
    );
    if !force && !needs_publish(prev.as_ref(), &snap) {
        return;
    }

    // The cover goes in before the snapshot that refers to it. A cover that
    // can't be fetched is removed rather than left over from the last track.
    let thumb = track.as_ref().and_then(|(_, t)| t.thumb.clone());
    let shown = inner.art_thumb.lock().clone();
    let source = match files::art_step(&snap, thumb.as_deref(), shown.as_deref()) {
        ArtStep::Keep => None,
        ArtStep::Copy(t) => Some((t.clone(), art_path(&state, t).await)),
        ArtStep::Remove => Some((String::new(), None)),
    };
    if let Some((t, path)) = source {
        let copied = match path {
            Some(p) => files::write_art(&dir, Some(p.as_path())).is_ok(),
            None => false,
        };
        if !copied {
            if let Err(e) = files::write_art(&dir, None) {
                log::warn!("widget: removing cover failed: {e}");
            }
        }
        *inner.art_thumb.lock() = copied.then_some(t);
    }

    match files::write_snapshot(&dir, &snap) {
        Ok(()) => reload_widget(app),
        Err(e) => log::warn!("widget: writing snapshot failed: {e}"),
    }
}

fn reload_widget(app: &AppHandle) {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        if let Err(e) = app.ramus_ios_bridge().widget_reload() {
            log::warn!("widget: reload failed: {e}");
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = app;
}

fn container_dir(app: &AppHandle) -> Option<PathBuf> {
    #[cfg(target_os = "ios")]
    {
        use tauri_plugin_ramus_ios_bridge::RamusIosBridgeExt;
        match app.ramus_ios_bridge().widget_container_path() {
            Ok(path) => path.map(PathBuf::from),
            Err(e) => {
                log::warn!("widget: no App Group container: {e}");
                None
            }
        }
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = app;
        None
    }
}

async fn album_colors(
    state: &tauri::State<'_, AppState>,
    track: &Track,
) -> Option<ramus_core::widget::WidgetColors> {
    let info = match &track.album_key {
        Some(key) => library::get_album_colors(state.clone(), key.clone())
            .await
            .ok(),
        None => None,
    };
    Some(widget_colors(info.as_ref(), track.thumb.as_deref()))
}

async fn waveform_for(
    inner: &Inner,
    state: &tauri::State<'_, AppState>,
    track: &Track,
) -> Option<Vec<f32>> {
    if let Some((key, levels)) = inner.waveform.lock().as_ref() {
        if *key == track.rating_key {
            return levels.clone();
        }
    }
    let fetch = playback::get_waveform(state.clone(), track.rating_key.clone());
    // Only an answer is remembered; a failed or slow fetch is retried on
    // the next publish.
    let levels = match tokio::time::timeout(FETCH_TIMEOUT, fetch).await {
        Ok(Ok(levels)) => levels.map(|l| downsample_levels(&l, WAVEFORM_POINTS)),
        _ => return None,
    };
    *inner.waveform.lock() = Some((track.rating_key.clone(), levels.clone()));
    levels
}

/// The cached file for a cover, fetching it if needed.
async fn art_path(state: &tauri::State<'_, AppState>, thumb: String) -> Option<PathBuf> {
    let fetch = library::get_art_url(state.clone(), thumb, Some(1200));
    match tokio::time::timeout(FETCH_TIMEOUT, fetch).await {
        Ok(Ok(path)) => Some(PathBuf::from(path)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_file_contains_no_urls() {
        let track = Track {
            rating_key: "9".into(),
            title: "Tessellate".into(),
            artist_name: "alt-J".into(),
            track_artist: None,
            album_title: "An Awesome Wave".into(),
            album_key: Some("99".into()),
            index: Some(3),
            duration: 182.7,
            codec: Some("flac".into()),
            part_key: Some("/library/parts/1/file.flac?X-Plex-Token=abc".into()),
            thumb: Some("/library/metadata/1/thumb?X-Plex-Token=abc".into()),
            is_favourite: false,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count: None,
            view_count: None,
            last_viewed_at: None,
        };
        let snap = build_snapshot(
            SnapshotInput {
                signed_in: true,
                status: PlaybackStatus::Playing,
                track: Some(widget_track(&track, track.duration)),
                position_sec: 1.0,
                has_queue: true,
                resume_enabled: true,
                colors: None,
                waveform: None,
                cast_player: None,
                now: 1.0,
            },
            None,
        );
        let json = serde_json::to_string(&snap).unwrap();
        assert!(!json.contains("X-Plex-Token"), "{json}");
        assert!(!json.contains("http"), "{json}");
        assert!(!json.contains("/library/"), "{json}");
    }
}
