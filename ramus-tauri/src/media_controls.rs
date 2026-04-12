//! System media controls integration via souvlaki.
//!
//! Provides OS-level Now Playing display (album art, title, transport buttons)
//! and routes hardware media key events back to the player.
//! - macOS: MPRemoteCommandCenter + MPNowPlayingInfoCenter
//! - Windows: System Media Transport Controls (SMTC)
//! - Linux: MPRIS2 over D-Bus

use std::sync::Arc;
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaPlayback, MediaPosition, PlatformConfig,
};

use ramus_core::cache::image_cache::ImageCache;
use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::player::AudioPlayer;
use ramus_core::plex::client::PlexClient;

/// Deferred-population slot for media controls, matching the pattern used
/// by `ReporterRef` and `PrefetchHandleRef`.
pub type MediaControlsRef = Arc<parking_lot::Mutex<Option<MediaControlsHandle>>>;

/// Sizes the frontend caches art at (from ui/src/lib/commands.ts ART_SIZE).
/// Try in this order: MEDIUM (album grid, most common), LARGE (Now Playing),
/// SMALL (search/queue).
const ART_SIZES_TO_TRY: &[u32] = &[300, 1200, 72];

/// Size we download at when nothing is cached yet.
const ART_DOWNLOAD_SIZE: u32 = 300;

/// Cached art resolution state — kept in a single mutex so the thumb and
/// its resolved URL are always consistent (no torn reads).
#[derive(Default)]
struct CachedArt {
    thumb: Option<String>,
    cover_url: Option<String>,
}

/// Wrapper around souvlaki's `MediaControls` implementing `MediaKeyHandler`.
pub struct MediaControlsHandle {
    controls: Arc<parking_lot::Mutex<MediaControls>>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
    cached_art: Arc<parking_lot::Mutex<CachedArt>>,
}

// souvlaki::MediaControls is Send + Sync (dispatches internally on macOS),
// so MediaControlsHandle auto-derives Send + Sync from its fields.

impl MediaControlsHandle {
    /// Resolve album art from the image cache, returning a `file:///` URL.
    /// Tries all frontend-cached sizes before giving up.
    fn resolve_cover_art(&self, thumb: Option<&str>) -> Option<String> {
        let thumb = thumb?;

        // Fast path: already resolved this thumb successfully
        {
            let cached = self.cached_art.lock();
            if cached.thumb.as_deref() == Some(thumb) && cached.cover_url.is_some() {
                return cached.cover_url.clone();
            }
            // Either different thumb or previous lookup was a miss — retry,
            // the background download may have populated the cache since then.
        }

        // Try each size the frontend may have cached
        let path = {
            let mut cache = self.image_cache.lock();
            ART_SIZES_TO_TRY
                .iter()
                .find_map(|&size| cache.get(thumb, size))
        };

        let cover_url = path.map(|p| format!("file://{}", p.display()));

        let mut cached = self.cached_art.lock();
        cached.thumb = Some(thumb.to_string());
        cached.cover_url = cover_url.clone();

        cover_url
    }
}

impl MediaKeyHandler for MediaControlsHandle {
    fn update_metadata(&self, metadata: &MediaMetadata) {
        let cover_url = self.resolve_cover_art(metadata.cover_url.as_deref());
        push_to_souvlaki(&self.controls, metadata, cover_url.as_deref());

        // If art wasn't in the cache, download it from Plex in the background
        // and re-push metadata once it arrives (same as Swift NowPlayingBridge).
        if cover_url.is_none() {
            if let Some(ref thumb) = metadata.cover_url {
                spawn_art_download(
                    thumb.clone(),
                    metadata.clone(),
                    self.controls.clone(),
                    self.image_cache.clone(),
                    self.client.clone(),
                    self.http_client.clone(),
                    self.cached_art.clone(),
                );
            }
        }
    }

    fn update_playback_state(&self, is_playing: bool, position: f64) {
        let progress = Duration::from_secs_f64(position.max(0.0));
        let mut controls = self.controls.lock();
        let _ = controls.set_playback(if is_playing {
            MediaPlayback::Playing {
                progress: Some(MediaPosition(progress)),
            }
        } else {
            MediaPlayback::Paused {
                progress: Some(MediaPosition(progress)),
            }
        });
    }

    fn clear(&self) {
        let mut controls = self.controls.lock();
        let _ = controls.set_playback(MediaPlayback::Stopped);
        let _ = controls.set_metadata(souvlaki::MediaMetadata {
            title: None,
            artist: None,
            album: None,
            cover_url: None,
            duration: None,
        });
        *self.cached_art.lock() = CachedArt::default();
    }
}

/// Push metadata + playback state to souvlaki. Shared between the sync
/// path (update_metadata) and the async art-download completion.
fn push_to_souvlaki(
    controls: &parking_lot::Mutex<MediaControls>,
    metadata: &MediaMetadata,
    cover_url: Option<&str>,
) {
    let duration = Duration::from_secs_f64(metadata.duration);
    let progress = Duration::from_secs_f64(metadata.position.max(0.0));

    let souvlaki_meta = souvlaki::MediaMetadata {
        title: Some(&metadata.title),
        artist: Some(&metadata.artist),
        album: Some(&metadata.album),
        cover_url,
        duration: Some(duration),
    };

    let mut ctl = controls.lock();
    let _ = ctl.set_metadata(souvlaki_meta);
    let _ = ctl.set_playback(if metadata.is_playing {
        MediaPlayback::Playing {
            progress: Some(MediaPosition(progress)),
        }
    } else {
        MediaPlayback::Paused {
            progress: Some(MediaPosition(progress)),
        }
    });
}

/// Download album art from Plex in the background, insert into the image
/// cache, then re-push metadata to souvlaki with the cover URL.
///
/// Guards against stale downloads: if the track changes while the download
/// is in flight (detected via `cached_art.thumb`), the result is discarded.
fn spawn_art_download(
    thumb: String,
    metadata: MediaMetadata,
    controls: Arc<parking_lot::Mutex<MediaControls>>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
    cached_art: Arc<parking_lot::Mutex<CachedArt>>,
) {
    tauri::async_runtime::spawn(async move {
        let server_url = match client.server_url() {
            Some(u) => u,
            None => return,
        };
        let token = match client.token() {
            Some(t) => t,
            None => return,
        };

        let url = format!(
            "{}/photo/:/transcode?width={}&height={}&minSize=1&upscale=1&url={}",
            server_url.as_str().trim_end_matches('/'),
            ART_DOWNLOAD_SIZE,
            ART_DOWNLOAD_SIZE,
            urlencoding::encode(&thumb),
        );

        let response = match http_client
            .get(&url)
            .header("X-Plex-Token", &token)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(_) => return,
        };

        // Insert into cache and get the file path
        let cover_url = {
            let mut cache = image_cache.lock();
            cache
                .insert(&thumb, ART_DOWNLOAD_SIZE, &bytes)
                .ok()
                .map(|p| format!("file://{}", p.display()))
        };

        // Only re-push if this thumb is still current (guard against rapid skipping
        // overwriting the new track's metadata with the old track's art).
        if let Some(ref url) = cover_url {
            let mut art = cached_art.lock();
            if art.thumb.as_deref() == Some(&thumb) {
                art.cover_url = Some(url.clone());
                drop(art);
                push_to_souvlaki(&controls, &metadata, Some(url));
            }
        }
    });
}

/// Create and configure OS media controls.
///
/// Returns `Err` if the platform doesn't support media controls (e.g., no
/// D-Bus on Linux, no HWND on Windows). Callers should treat this as non-fatal.
pub fn create_media_controls(
    #[cfg(target_os = "windows")] window: &tauri::WebviewWindow,
    player: Arc<AudioPlayer>,
    image_cache: Arc<parking_lot::Mutex<ImageCache>>,
    client: Arc<PlexClient>,
    http_client: reqwest::Client,
) -> Result<MediaControlsHandle, String> {
    #[cfg(target_os = "windows")]
    let hwnd = {
        // Tauri exposes .hwnd() directly on Windows — same pattern as
        // .ns_window() on macOS in main.rs. No need for raw_window_handle crate.
        let h = window
            .hwnd()
            .map_err(|e| format!("failed to get HWND: {e}"))?;
        Some(h.0 as *mut std::ffi::c_void)
    };

    let config = PlatformConfig {
        dbus_name: "ramus",
        display_name: "Ramus",
        #[cfg(target_os = "windows")]
        hwnd,
        #[cfg(not(target_os = "windows"))]
        hwnd: None,
    };

    let mut controls =
        MediaControls::new(config).map_err(|e| format!("failed to create media controls: {e}"))?;

    // Attach event handler that routes OS media key events to the player
    let p = player.clone();
    controls
        .attach(move |event: MediaControlEvent| {
            match event {
                MediaControlEvent::Play => p.resume(),
                MediaControlEvent::Pause => p.pause(),
                MediaControlEvent::Toggle => p.toggle_play_pause(),
                MediaControlEvent::Next => p.next(),
                MediaControlEvent::Previous => p.previous(),
                MediaControlEvent::Stop => p.stop(),
                MediaControlEvent::SetPosition(MediaPosition(dur)) => {
                    p.seek(dur.as_secs_f64());
                }
                MediaControlEvent::SetVolume(vol) => {
                    // souvlaki volume is 0.0–1.0, player expects 0–100
                    p.set_volume(vol * 100.0);
                }
                // SeekBy, Seek, OpenUri, Raise, Quit — not applicable
                _ => {}
            }
        })
        .map_err(|e| format!("failed to attach media control handler: {e}"))?;

    Ok(MediaControlsHandle {
        controls: Arc::new(parking_lot::Mutex::new(controls)),
        image_cache,
        client,
        http_client,
        cached_art: Arc::new(parking_lot::Mutex::new(CachedArt::default())),
    })
}
