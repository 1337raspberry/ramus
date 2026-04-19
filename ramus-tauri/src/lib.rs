pub mod auto_sync;
pub mod commands;
pub mod events;

// Desktop (macOS/Windows/Linux) uses souvlaki for system media controls;
// iOS gets a Swift-plugin bridge to MPNowPlayingInfoCenter + MPRemoteCommandCenter.
// Android currently has a no-op stub — Media3/MediaSession integration is TBD.
#[cfg(desktop)]
pub mod media_controls;
#[cfg(target_os = "ios")]
#[path = "media_controls_ios.rs"]
pub mod media_controls;
#[cfg(target_os = "android")]
#[path = "media_controls_android.rs"]
pub mod media_controls;

// libmpv is loaded at runtime via `libloading` on desktop (see mpv_ffi.rs).
// iOS uses `mpv_ios.rs` — delegates to the Swift plugin's MPVKit handle via
// Tauri IPC. Android has a no-op player stub until mpv-android .so bundling
// or a Media3-based backend lands.
#[cfg(target_os = "ios")]
pub mod keychain_ios;
#[cfg(desktop)]
pub mod mpv_controller;
#[cfg(desktop)]
pub mod mpv_ffi;
#[cfg(target_os = "ios")]
pub mod mpv_ios;
#[cfg(target_os = "android")]
pub mod mpv_android;

pub mod ios_backup;
pub mod prefetch;
pub mod session_reporter;
pub mod spectrum_analyzer;
pub mod state;

/// Cheap "is the internet reachable" probe. Races bare-TCP connects to
/// well-known, always-on endpoints (Cloudflare 1.1.1.1 and Google 8.8.8.8).
/// First success wins; if both time out within `per_host_timeout`, the
/// machine is genuinely offline and there's no point grinding through a
/// 30s Plex discovery. Much faster than HTTP since we skip TLS entirely.
async fn internet_reachable(per_host_timeout: std::time::Duration) -> bool {
    let candidates = ["1.1.1.1:443", "8.8.8.8:443"];
    let mut set = tokio::task::JoinSet::new();
    for addr in candidates {
        set.spawn(async move {
            tokio::time::timeout(per_host_timeout, tokio::net::TcpStream::connect(addr))
                .await
                .is_ok_and(|r| r.is_ok())
        });
    }
    while let Some(res) = set.join_next().await {
        if let Ok(true) = res {
            return true;
        }
    }
    false
}

use std::sync::Arc;

use tauri::AppHandle;

use ramus_core::playback::media_keys::{MediaKeyHandler, MediaMetadata};
use ramus_core::playback::mpv::MpvCallbacks;

use crate::events::{
    emit_playback_position, emit_playback_state, PlaybackPositionPayload, PlaybackStatePayload,
};
use crate::media_controls::MediaControlsRef;
#[cfg(desktop)]
use crate::mpv_controller::MpvController;
#[cfg(desktop)]
use crate::mpv_ffi::MpvLib;
use crate::prefetch::PrefetchHandle;
use crate::session_reporter::ReporterRef;

/// Deferred-population slot for the prefetch control handle. `main.rs` builds
/// the empty slot and populates it after `spawn_worker()` returns; mpv callback
/// closures read the slot to pump commands (natural-advance / skip / cancel)
/// into the worker.
pub type PrefetchHandleRef = Arc<parking_lot::Mutex<Option<PrefetchHandle>>>;

/// Create an AudioPlayer backed by libmpv with Tauri event callbacks.
///
/// `prefetch_handle_ref` is a deferred slot: `main.rs` passes an empty
/// `Arc<Mutex<None>>` and populates it after `spawn_worker()` returns.
///
/// Desktop builds construct a real `MpvController` via `libloading` +
/// the shared mpv handle. iOS builds construct an `IosMpvPlayer` that
/// delegates to the Swift plugin (MPVKit). The callback wiring is
/// identical on both platforms, so only the final construction diverges.
pub fn create_mpv_player(
    app_handle: AppHandle,
    prefetch_handle_ref: PrefetchHandleRef,
) -> (
    Arc<ramus_core::playback::player::AudioPlayer>,
    ReporterRef,
    MediaControlsRef,
) {
    let app1 = app_handle.clone();
    let app2 = app_handle.clone();
    let app3 = app_handle.clone();
    let app4 = app_handle.clone();
    let app7 = app_handle.clone();

    // The player is needed inside callbacks but owns the MpvController. A
    // shared Arc populated after construction breaks the cycle.
    let player_ref: Arc<
        parking_lot::Mutex<Option<Arc<ramus_core::playback::player::AudioPlayer>>>,
    > = Arc::new(parking_lot::Mutex::new(None));
    let pr1 = player_ref.clone();
    let pr2 = player_ref.clone();
    let pr3 = player_ref.clone();
    let pr4 = player_ref.clone();
    let pr7 = player_ref.clone();
    let pr8 = player_ref.clone();
    let pr9 = player_ref.clone();

    // Deferred session reporter; populated after player construction.
    let reporter_ref: ReporterRef = Arc::new(parking_lot::Mutex::new(None));
    let sr1 = reporter_ref.clone();
    let sr2 = reporter_ref.clone();
    let sr3 = reporter_ref.clone();

    // Deferred media controls; populated after window creation in main.rs.
    let media_controls_ref: MediaControlsRef = Arc::new(parking_lot::Mutex::new(None));
    let mc1 = media_controls_ref.clone();
    let mc2 = media_controls_ref.clone();
    let mc3 = media_controls_ref.clone();

    let ph1 = prefetch_handle_ref.clone();
    let ph2 = prefetch_handle_ref.clone();

    let callbacks = Arc::new(MpvCallbacks {
        on_position_change: Some(Box::new(move |pos| {
            if let Some(ref p) = *pr1.lock() {
                p.handle_position_change(pos);
                let dur = p.duration();
                emit_playback_position(
                    &app1,
                    PlaybackPositionPayload {
                        position: pos,
                        duration: dur,
                    },
                );
            }
        })),
        on_duration_change: Some(Box::new(move |dur| {
            if let Some(ref p) = *pr2.lock() {
                let old_dur = p.duration();
                p.handle_duration_change(dur);
                // Emit immediately so the frontend gets the new duration without
                // waiting for the next time-pos tick. Use position 0 on a track
                // boundary (previous duration 0) to avoid pairing the old track's
                // position with the new track's duration during prefetch transitions.
                let pos = if old_dur == 0.0 { 0.0 } else { p.position() };
                emit_playback_position(
                    &app2,
                    PlaybackPositionPayload {
                        position: pos,
                        duration: dur,
                    },
                );

                // Push full metadata to OS media controls once the real duration
                // is known (fires shortly after track change).
                if let Some(ref mc) = *mc1.lock() {
                    if let Some(ref track) = p.state().current_track {
                        let meta = MediaMetadata::from_track(track, pos, dur, true);
                        mc.update_metadata(&meta);
                    }
                }
            }
        })),
        on_playlist_pos_change: Some(Box::new(move |pos| {
            if let Some(ref p) = *pr3.lock() {
                // Capture previous track before state update for scrobble reporting.
                let prev_track = p.state().current_track.clone();

                p.handle_playlist_pos_change(pos);
                let state = p.state();
                emit_playback_state(
                    &app3,
                    PlaybackStatePayload {
                        status: format!("{:?}", state.status).to_lowercase(),
                        current_track: state.current_track.clone(),
                        queue_index: state.queue_index,
                    },
                );

                // Nudge the prefetch worker: a natural advance shifts the
                // lookahead window, potentially bringing a new uncached target
                // into scope. The worker starts a fresh cycle if idle, or lets
                // the running serial loop pick up the shift on its next iteration.
                if let Some(ref handle) = *ph1.lock() {
                    handle.notify_natural_advance();
                }

                // Session reporting for natural track advance only. Matching
                // rating_key means a queue reload, which play_tracks already
                // reported via track_started.
                if let Some(ref reporter) = *sr1.lock() {
                    if let Some(ref prev) = prev_track {
                        let same_track = state
                            .current_track
                            .as_ref()
                            .is_some_and(|cur| cur.rating_key == prev.rating_key);
                        if !same_track {
                            reporter.playback_stopped();
                            reporter.track_ended(prev);
                            if let Some(ref track) = state.current_track {
                                reporter.track_started(track, &p.play_session_id());
                            }
                        }
                    }
                }
            }
        })),
        on_pause_change: Some(Box::new(move |paused| {
            if let Some(ref p) = *pr4.lock() {
                p.handle_pause_change(paused);
                let state = p.state();
                emit_playback_state(
                    &app4,
                    PlaybackStatePayload {
                        status: if paused {
                            "paused".to_string()
                        } else {
                            "playing".to_string()
                        },
                        current_track: state.current_track,
                        queue_index: state.queue_index,
                    },
                );

                if let Some(ref reporter) = *sr2.lock() {
                    if paused {
                        reporter.playback_paused();
                    } else {
                        reporter.playback_resumed();
                    }
                }

                if let Some(ref mc) = *mc2.lock() {
                    mc.update_playback_state(!paused, p.position());
                }
            }
        })),
        on_idle_active: Some(Box::new(move || {
            if let Some(ref p) = *pr7.lock() {
                // Scrobble the last playing track before transitioning to stopped.
                if let Some(ref reporter) = *sr3.lock() {
                    if let Some(ref track) = p.state().current_track {
                        reporter.track_ended(track);
                    }
                }

                p.handle_idle_active();
                emit_playback_state(
                    &app7,
                    PlaybackStatePayload {
                        status: "stopped".to_string(),
                        current_track: None,
                        queue_index: 0,
                    },
                );

                if let Some(ref reporter) = *sr3.lock() {
                    reporter.playback_stopped();
                }

                if let Some(ref mc) = *mc3.lock() {
                    mc.clear();
                }

                // Queue finished; stop the prefetch worker until the next queue loads.
                if let Some(ref handle) = *ph2.lock() {
                    handle.notify_cancel();
                }
            }
        })),
        on_file_loaded: Some(Box::new(move || {
            if let Some(ref p) = *pr8.lock() {
                p.handle_file_loaded();
            }
        })),
        on_file_ended: Some(Box::new(move |reason| {
            if let Some(ref p) = *pr9.lock() {
                p.handle_file_ended(reason);
            }
        })),
    });

    // Platform split: desktop loads libmpv at runtime via `libloading`;
    // iOS talks to the MPVKit-backed Swift plugin through Tauri IPC;
    // Android currently uses a no-op stub player. The `AudioPlayer`
    // surface is identical across all three.
    #[cfg(desktop)]
    let player = {
        // `MpvLib::load()` returns a multi-line string listing every path
        // it tried; surface that verbatim if it fails.
        let mpv_lib = Arc::new(MpvLib::load().unwrap_or_else(|e| panic!("{e}")));
        let mpv = MpvController::new(mpv_lib, callbacks).expect("Failed to initialize libmpv");
        Arc::new(ramus_core::playback::player::AudioPlayer::new(Arc::new(
            mpv,
        )))
    };
    #[cfg(target_os = "ios")]
    let player = {
        let ios_mpv = crate::mpv_ios::IosMpvPlayer::new(app_handle.clone(), callbacks)
            .expect("failed to initialise iOS mpv bridge");
        Arc::new(ramus_core::playback::player::AudioPlayer::new(Arc::new(
            ios_mpv,
        )))
    };
    #[cfg(target_os = "android")]
    let player = {
        let _ = app_handle;
        let android_mpv = crate::mpv_android::AndroidMpvPlayer::new(callbacks);
        Arc::new(ramus_core::playback::player::AudioPlayer::new(Arc::new(
            android_mpv,
        )))
    };

    *player_ref.lock() = Some(player.clone());

    (player, reporter_ref, media_controls_ref)
}

/// Adapter letting `SearchEngine` read the genre mapper through the shared
/// `RwLock`. Reads lazily — returns `None` before the mapper loads,
/// equivalent to passing no expander at all.
struct SharedGenreExpander {
    mapper: Arc<parking_lot::RwLock<Option<ramus_core::genre::mapper::GenreMapper>>>,
}

impl ramus_core::search::engine::GenreExpander for SharedGenreExpander {
    fn expand_genre(&self, name: &str) -> Option<std::collections::HashSet<String>> {
        self.mapper.read().as_ref()?.expand_genre(name)
    }
}

/// Tauri app entry point, shared by the desktop binary (`main.rs`) and the
/// iOS static-library build (`#[tauri::mobile_entry_point]` exports it as
/// `start_app` for the generated Xcode project).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use parking_lot::RwLock;
    use tauri::Manager;

    use ramus_core::cache::db::CacheDatabase;
    use ramus_core::cache::sync::SyncEngine;
    use ramus_core::genre::mapper::GenreMapper;
    use ramus_core::models::PlexServer;
    use ramus_core::plex::auth;
    use ramus_core::plex::client::PlexClient;
    use ramus_core::plex::connection::ConnectionMonitor;
    use ramus_core::plex::token_store::{TokenKey, TokenStore};
    use ramus_core::search::engine::SearchEngine;

    use crate::state::AppState;

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("ramus_tauri=debug,ramus_core=debug,info"),
    )
    .format_timestamp_millis()
    .init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_ramus_ios_bridge::init());

    // Window-state persistence is only meaningful on desktop — mobile has no
    // resizable window, and the plugin crate isn't linked on mobile targets.
    #[cfg(desktop)]
    let builder = builder.plugin(
        tauri_plugin_window_state::Builder::new()
            .with_state_flags(
                // POSITION excluded: upstream bug tauri-apps/tauri#14822 hangs
                // with decorations:false on macOS, and borderless position
                // restore is unreliable on Windows.
                tauri_plugin_window_state::StateFlags::SIZE
                    | tauri_plugin_window_state::StateFlags::MAXIMIZED,
            )
            .build(),
    );

    builder
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Android: seed the config-dir override before anything in
            // ramus-core touches `config_dir()`. The `directories` crate
            // returns `None` on Android, so `ProjectDirs::from` would
            // otherwise fail with `NoConfigDir`. Tauri's `PathResolver`
            // knows the right per-package sandbox path
            // (`/data/user/0/<package>/files/`).
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                match app_handle.path().app_data_dir() {
                    Ok(dir) => ramus_core::plex::token_store::set_config_dir(dir),
                    Err(e) => log::error!("android: app_data_dir lookup failed: {e}"),
                }
            }

            // Register the iOS keychain backend before any `TokenStore::new()`
            // call. On desktop this is a no-op — `TokenStore` uses the
            // file+AES backend and never touches the global slot.
            #[cfg(target_os = "ios")]
            crate::keychain_ios::register(&app_handle);

            // Register the iOS backup-exclusion backend so downloaded audio
            // files stay out of iCloud / iTunes backups. Desktop is a no-op.
            #[cfg(target_os = "ios")]
            crate::ios_backup::register_ios(&app_handle);

            // Force-fit the UIWindow to the full screen on iOS. The shared
            // `tauri.conf.json` sets `width: 1200, height: 800` for desktop,
            // which tao's iOS window constructor respects verbatim, leaving
            // an empty strip below the app. tao's `set_inner_size` is a
            // no-op on iOS and `WebviewWindow::set_fullscreen` only compiles
            // on `cfg(desktop)`, so we reach into the WKWebView's view
            // controller via `with_webview`, walk up to the UIWindow, and
            // resize it with `setFrame: [[UIScreen mainScreen] bounds]`.
            #[cfg(target_os = "ios")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|pw| {
                        // Capture as usize so the closure is Send + 'static; we
                        // cast back to pointers on the main thread inside the
                        // dispatched block.
                        let vc_ptr = pw.view_controller() as usize;
                        let webview_ptr = pw.inner() as usize;

                        dispatch2::DispatchQueue::main().exec_async(move || unsafe {
                            use objc2::msg_send;
                            use objc2::runtime::AnyObject;
                            use objc2_core_foundation::CGRect;

                            let vc = vc_ptr as *mut AnyObject;
                            let webview = webview_ptr as *mut AnyObject;
                            if vc.is_null() || webview.is_null() {
                                return;
                            }
                            let root_view: *mut AnyObject = msg_send![vc, view];
                            if root_view.is_null() {
                                return;
                            }
                            let win: *mut AnyObject = msg_send![root_view, window];
                            if win.is_null() {
                                return;
                            }
                            let uiscreen_cls = objc2::runtime::AnyClass::get(
                                std::ffi::CStr::from_bytes_with_nul_unchecked(b"UIScreen\0"),
                            );
                            let Some(uiscreen_cls) = uiscreen_cls else {
                                return;
                            };
                            let main_screen: *mut AnyObject = msg_send![uiscreen_cls, mainScreen];
                            if main_screen.is_null() {
                                return;
                            }
                            let bounds: CGRect = msg_send![main_screen, bounds];

                            // Flexible W|H so future resizes (rotation, keyboard)
                            // propagate through all three layers.
                            // 2 = UIViewAutoresizingFlexibleWidth
                            // 16 = UIViewAutoresizingFlexibleHeight
                            let flexible: u64 = 2 | 16;
                            let _: () = msg_send![root_view, setAutoresizingMask: flexible];
                            let _: () = msg_send![
                                webview,
                                setTranslatesAutoresizingMaskIntoConstraints: true
                            ];
                            let _: () = msg_send![webview, setAutoresizingMask: flexible];

                            let _: () = msg_send![win, setFrame: bounds];
                            let _: () = msg_send![root_view, setFrame: bounds];
                            let _: () = msg_send![webview, setFrame: bounds];

                            // Disable the scrollView's automatic content-inset
                            // adjustment. By default iOS sets it to `Automatic`,
                            // which inserts padding for the safe area on top of
                            // our CSS — and the scrollView only recomputes that
                            // padding on a layout event (keyboard focus, rotate).
                            // With `viewport-fit=cover` + explicit
                            // `env(safe-area-inset-*)` CSS, we own the safe-area
                            // handling and iOS should stay out of it.
                            //   Never = 2
                            let scroll_view: *mut AnyObject = msg_send![webview, scrollView];
                            if !scroll_view.is_null() {
                                let never: i64 = 2;
                                let _: () = msg_send![
                                    scroll_view,
                                    setContentInsetAdjustmentBehavior: never
                                ];
                            }

                            // Force the scrollView inside WKWebView to recompute
                            // its viewport + safe-area insets. Without this,
                            // CSS `100dvh` and `env(safe-area-inset-*)` return
                            // stale values until UIKit fires a layout event
                            // (keyboard focus, rotation).
                            let _: () = msg_send![root_view, setNeedsLayout];
                            let _: () = msg_send![webview, setNeedsLayout];
                            let _: () = msg_send![root_view, layoutIfNeeded];
                            let _: () = msg_send![webview, layoutIfNeeded];
                        });
                    });
                }
            }

            // Restore persistent client_identifier, or generate a new one.
            let id_path = ramus_core::plex::token_store::config_dir()
                .ok()
                .map(|d| d.join("client_id.txt"));

            let client_identifier = id_path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_else(|| {
                    let id = uuid::Uuid::new_v4().to_string();
                    if let Some(ref p) = id_path {
                        let _ = std::fs::create_dir_all(p.parent().unwrap());
                        let _ = std::fs::write(p, &id);
                    }
                    id
                });

            let client = Arc::new(PlexClient::new(client_identifier));
            let connection_monitor = Arc::new(ConnectionMonitor::new(client.clone()));
            let http_client = reqwest::Client::new();

            // Dedicated prefetch HTTP client with a 300s per-request timeout so
            // large FLAC files on slower LAN segments don't time out mid-download.
            // Separate from the app-wide client so prefetch's retry profile
            // doesn't leak into metadata fetches.
            let prefetch_http_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .tcp_nodelay(true)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            let prefetch_handle_ref: Arc<
                parking_lot::Mutex<Option<crate::prefetch::PrefetchHandle>>,
            > = Arc::new(parking_lot::Mutex::new(None));
            let (player, reporter_ref, media_controls_ref) =
                create_mpv_player(app_handle.clone(), prefetch_handle_ref.clone());

            // Spawn the long-lived prefetch worker and wire its control handle
            // back into the callbacks.
            let prefetch_handle = crate::prefetch::spawn_worker(
                player.clone(),
                prefetch_http_client,
                app_handle.clone(),
            );
            *prefetch_handle_ref.lock() = Some(prefetch_handle.clone());

            let session_reporter =
                crate::session_reporter::SessionReporter::new(client.clone(), player.clone());
            *reporter_ref.lock() = Some(session_reporter.clone());

            // Load saved settings and apply playback config (defaults to DirectPlay).
            #[allow(unused_mut)]
            let mut saved_settings = ramus_core::settings::load();
            #[cfg(target_os = "ios")]
            {
                saved_settings.disable_spectrum = true;
            }
            player.update_config(saved_settings.to_playback_config());
            player.apply_equalizer(saved_settings.eq_enabled, &saved_settings.eq_bands);

            let image_cache_dir = ramus_core::plex::token_store::config_dir()
                .map(|d| d.join("image_cache"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/ramus_image_cache"));
            let image_cache = ramus_core::cache::image_cache::ImageCache::load(
                image_cache_dir,
                saved_settings.image_cache_limit_bytes as u64,
            );

            let image_cache_arc = Arc::new(parking_lot::Mutex::new(image_cache));

            let state = AppState {
                client: client.clone(),
                cache: Arc::new(parking_lot::Mutex::new(None)),
                player: player.clone(),
                genre_mapper: Arc::new(RwLock::new(None)),
                search_engine: Arc::new(RwLock::new(None)),
                sync_engine: Arc::new(parking_lot::Mutex::new(None)),
                session_reporter: session_reporter.clone(),
                connection_monitor: connection_monitor.clone(),
                settings: Arc::new(RwLock::new(saved_settings)),
                image_cache: image_cache_arc.clone(),
                http_client: http_client.clone(),
                prefetch_handle,
                discovered_servers: Arc::new(parking_lot::Mutex::new(Vec::new())),
                media_controls: media_controls_ref.clone(),
                sync_in_progress: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                // Optimistic default — flipped to false by the startup probe
                // if no server answers, or by on_connection_lost callbacks
                // from the connection monitor.
                server_reachable: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            };

            // Restore previous session. State is set synchronously (no blocking
            // network call) so the window appears immediately; a background
            // task verifies connectivity and tests alternative connections.
            if let Ok(token_store) = TokenStore::new() {
                if let Some(config) = auth::stored_server_config(&token_store) {
                    // Pick a usable starting URL. Prefer the stored activeUri
                    // when it passes the refuse_http gate, otherwise fall back
                    // to the highest-priority connection in the stored list
                    // that does. Falling back (instead of skipping the whole
                    // restore) matters: the probe has added stale / per-boot
                    // URIs to activeUri in the past (loopback ports, relay
                    // URIs that expired), and a bogus activeUri shouldn't
                    // lock the user out of their own library when the
                    // connection list still has a valid HTTPS entry.
                    let settings_snapshot = state.settings.read().clone();
                    let allow_http = !settings_snapshot.refuse_http;
                    let usable = |u: &str| {
                        url::Url::parse(u)
                            .ok()
                            .filter(|p| allow_http || p.scheme() == "https")
                    };
                    let chosen_url = config
                        .active_uri
                        .as_deref()
                        .and_then(usable)
                        .or_else(|| {
                            // activeUri unusable — take the highest-priority
                            // connection that passes. `sorted_connections`
                            // already prefers local/https.
                            ramus_core::models::PlexServer::from(&config)
                                .sorted_connections()
                                .iter()
                                .find_map(|c| usable(&c.uri))
                        });

                    if let Some(url) = chosen_url {
                        let url_str = url.as_str().trim_end_matches('/').to_string();
                        let settings = settings_snapshot;
                        let token = config.access_token.clone();
                                let client_id = client.client_identifier.clone();

                                client.set_server_url(Some(url.clone()));
                                client.set_token(Some(token.clone()));

                                player.configure(url.clone(), token, client_id);

                                if let Ok(cache_dir) = ramus_core::plex::token_store::config_dir() {
                                    let db_path = cache_dir.join("library_cache.db");
                                    if let Ok(db) = CacheDatabase::open(&db_path) {
                                        let db_arc = Arc::new(db);

                                        let sync = SyncEngine::new(db_arc.clone(), client.clone());
                                        *state.sync_engine.lock() = Some(sync);

                                        let expander = Arc::new(SharedGenreExpander {
                                            mapper: state.genre_mapper.clone(),
                                        });
                                        let search =
                                            SearchEngine::new(db_arc.clone(), Some(expander));
                                        *state.search_engine.write() = Some(search);

                                        if let Ok(db2) = CacheDatabase::open(&db_path) {
                                            crate::prefetch::rehydrate_persistent_downloads(
                                                &state.player,
                                                &db2,
                                            );
                                            *state.cache.lock() = Some(db2);
                                        }
                                    }
                                }

                                // Load genre mapper; prefer custom genres if configured.
                                let custom_mapper = (settings.genre_source == ramus_core::models::GenreSource::Custom)
                                    .then(ramus_core::settings::load_custom_genres)
                                    .flatten()
                                    .and_then(|data| GenreMapper::from_json_bytes(&data).ok());

                                let mapper = custom_mapper.or_else(|| {
                                    let open_json = include_bytes!("../data/open.json");
                                    GenreMapper::from_json_bytes(open_json).ok()
                                });
                                if let Some(m) = mapper {
                                    *state.genre_mapper.write() = Some(m);
                                }

                                // plex.tv auth token for monitor re-discovery.
                                let auth_token = token_store
                                    .read(TokenKey::AuthToken)
                                    .unwrap_or_default();

                                let allow_http = !settings.refuse_http;
                                let plex_server = PlexServer::from(&config);
                                connection_monitor.set_allow_http(allow_http);
                                let bg_auth_token = auth_token.clone();

                                // Update player when monitor switches connections.
                                let monitor_player = state.player.clone();
                                let monitor_reachable = state.server_reachable.clone();
                                let monitor_app = app_handle.clone();
                                let monitor_settings_for_changed = state.settings.clone();
                                connection_monitor.set_on_connection_changed(
                                    std::sync::Arc::new(move |url, token, is_local, _is_http| {
                                        let is_remote = !is_local;
                                        monitor_player.update_server_connection(url, token, is_remote);
                                        monitor_reachable
                                            .store(true, std::sync::atomic::Ordering::Release);
                                        let offline_manual =
                                            monitor_settings_for_changed.read().offline_mode;
                                        crate::events::emit_connection_status(
                                            &monitor_app,
                                            crate::events::ConnectionStatusPayload {
                                                online: true,
                                                offline_mode_manual: offline_manual,
                                                effective_offline: offline_manual,
                                            },
                                        );
                                        log::info!(
                                            "monitor: updated player connection (is_remote={})",
                                            is_remote,
                                        );
                                    }),
                                );

                                // Flip reachable=false when all connections fail.
                                let lost_reachable = state.server_reachable.clone();
                                let lost_app = app_handle.clone();
                                let lost_settings = state.settings.clone();
                                connection_monitor.set_on_connection_lost(std::sync::Arc::new(
                                    move || {
                                        lost_reachable
                                            .store(false, std::sync::atomic::Ordering::Release);
                                        let offline_manual = lost_settings.read().offline_mode;
                                        crate::events::emit_connection_status(
                                            &lost_app,
                                            crate::events::ConnectionStatusPayload {
                                                online: false,
                                                offline_mode_manual: offline_manual,
                                                effective_offline: true,
                                            },
                                        );
                                        log::info!("monitor: connection lost");
                                    },
                                ));

                                connection_monitor.start(
                                    plex_server.clone(),
                                    url_str.clone(),
                                    auth_token,
                                );

                                // Background: try local first, then stored URI, then full test.
                                let bg_client = client.clone();
                                let bg_url = url.clone();
                                let bg_token = config.access_token.clone();
                                let bg_monitor = connection_monitor.clone();
                                let bg_settings = state.settings.clone();
                                let bg_player = state.player.clone();
                                let bg_reachable = state.server_reachable.clone();
                                let bg_app = app_handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    let allow_http = !bg_settings.read().refuse_http;

                                    let apply_connection =
                                        |conn: &ramus_core::models::PlexServerConnection| -> bool {
                                            if let Ok(new_url) = url::Url::parse(&conn.uri) {
                                                if bg_client.token().is_some() {
                                                    bg_client.set_server_url(Some(new_url.clone()));
                                                    let is_remote = !conn.local;
                                                    bg_player.set_remote(is_remote);
                                                    log::info!(
                                                        "connected via {} (is_remote={})",
                                                        conn.uri, is_remote,
                                                    );
                                                    ramus_core::plex::auth::patch_stored_config(
                                                        None,
                                                        Some(&conn.uri),
                                                    );
                                                    bg_monitor.update_active_uri(conn.uri.clone());
                                                    return true;
                                                }
                                            }
                                            false
                                        };

                                    let mut failed: std::collections::HashSet<String> = std::collections::HashSet::new();
                                    // Normalize for comparison — url::Url adds a trailing slash.
                                    let normalize = |s: &str| s.trim_end_matches('/').to_string();

                                    // 1. Try local connections first (~2s timeout).
                                    let local_conns: Vec<_> = plex_server
                                        .sorted_connections()
                                        .into_iter()
                                        .filter(|c| c.local)
                                        .filter(|c| allow_http || c.protocol == "https")
                                        .collect();

                                    if !local_conns.is_empty() {
                                        log::debug!(
                                            "trying {} local connection(s) first",
                                            local_conns.len(),
                                        );
                                        for conn in &local_conns {
                                            if bg_client
                                                .test_connection(
                                                    &conn.uri,
                                                    &bg_token,
                                                    Some(std::time::Duration::from_secs(2)),
                                                )
                                                .await
                                            {
                                                let conn = (*conn).clone();
                                                if apply_connection(&conn) {
                                                    return;
                                                }
                                            }
                                            log::debug!("local connection failed: {}", conn.uri);
                                            failed.insert(normalize(&conn.uri));
                                        }
                                    }

                                    // 2. Try stored activeUri (skip if already tested as local).
                                    let stored_normalized = normalize(bg_url.as_str());
                                    if !failed.contains(&stored_normalized) {
                                        log::debug!("testing stored connection: {}", bg_url);
                                        if bg_client
                                            .test_connection(
                                                bg_url.as_str(),
                                                &bg_token,
                                                Some(std::time::Duration::from_secs(3)),
                                            )
                                            .await
                                        {
                                            let is_local = plex_server
                                                .connections
                                                .iter()
                                                .any(|c| normalize(&c.uri) == stored_normalized && c.local);
                                            bg_player.set_remote(!is_local);
                                            log::debug!(
                                                "stored connection ok (is_remote={})",
                                                !is_local,
                                            );
                                            return;
                                        }
                                        failed.insert(stored_normalized);
                                    } else {
                                        log::debug!("skipping stored connection (already failed): {}", bg_url);
                                    }

                                    // Abort if logged out during probes.
                                    if bg_client.token().is_none() {
                                        return;
                                    }

                                    // 3. Test remaining cached connections (skip already-failed).
                                    let remaining: Vec<_> = plex_server
                                        .connections
                                        .iter()
                                        .filter(|c| !failed.contains(&normalize(&c.uri)))
                                        .cloned()
                                        .collect();

                                    if remaining.is_empty() && !plex_server.connections.is_empty() {
                                        log::debug!("all cached connections already tested, skipping to re-discovery");
                                    } else if !remaining.is_empty() {
                                        log::info!(
                                            "testing {} remaining cached connection(s) (skipped {} already failed)",
                                            remaining.len(),
                                            failed.len(),
                                        );
                                        let filtered_server = ramus_core::models::PlexServer {
                                            connections: remaining,
                                            ..plex_server.clone()
                                        };
                                        let (best, _is_http) = bg_client
                                            .find_best_connection(&filtered_server, allow_http, true)
                                            .await;
                                        if let Some(conn) = best {
                                            if apply_connection(&conn) {
                                                return;
                                            }
                                        }
                                    }

                                    // 4. Re-discover from plex.tv. Skipped if
                                    // we can't reach it — no stored auth
                                    // token, or a cheap internet probe fails
                                    // (airplane mode, dead router). Either
                                    // way we fall through to the offline
                                    // emit at the end. LAN-only Plex setups
                                    // with no internet still got through
                                    // steps 1–3 above.
                                    let can_rediscover = if bg_auth_token.is_empty() {
                                        log::warn!("cannot re-discover from plex.tv — no auth token stored");
                                        false
                                    } else if !internet_reachable(std::time::Duration::from_secs(1)).await {
                                        log::info!(
                                            "startup probe: internet unreachable, skipping plex.tv re-discovery"
                                        );
                                        false
                                    } else {
                                        true
                                    };
                                    if can_rediscover {
                                        log::info!("no cached alternatives, re-discovering from plex.tv");
                                        // Cap re-discovery at 5s. The reqwest
                                        // default is ~30s which felt glacial
                                        // on a boot where plex.tv is slow.
                                        let rediscover = tokio::time::timeout(
                                            std::time::Duration::from_secs(5),
                                            bg_client.discover_servers(&bg_auth_token),
                                        )
                                        .await;
                                        if let Ok(Ok(servers)) = rediscover {
                                        if let Some(found) = servers
                                            .iter()
                                            .find(|s| s.machine_identifier == plex_server.machine_identifier)
                                        {
                                            log::debug!(
                                                "re-discovered server with {} connection(s)",
                                                found.connections.len(),
                                            );
                                            let (best, _is_http) = bg_client
                                                .find_best_connection(found, allow_http, true)
                                                .await;
                                            if let Some(conn) = best {
                                                if apply_connection(&conn) {
                                                    bg_monitor.update_server(found.clone());
                                                    ramus_core::plex::auth::patch_stored_config(
                                                        Some(&found.connections),
                                                        None,
                                                    );
                                                    // Connected via plex.tv — skip the
                                                    // fall-through offline emit below.
                                                    return;
                                                }
                                            } else {
                                                log::warn!(
                                                    "re-discovered server but all {} connection(s) failed",
                                                    found.connections.len(),
                                                );
                                            }
                                        } else {
                                            log::warn!(
                                                "server {} not found in {} re-discovered server(s)",
                                                plex_server.machine_identifier,
                                                servers.len(),
                                            );
                                        }
                                    } else {
                                        log::warn!("plex.tv re-discovery failed");
                                    }
                                    } // end if can_rediscover

                                    // Fall-through: none of the four probe
                                    // paths connected. Flip the reachability
                                    // flag and notify the UI so it can enter
                                    // offline mode.
                                    bg_reachable
                                        .store(false, std::sync::atomic::Ordering::Release);
                                    let offline_manual = bg_settings.read().offline_mode;
                                    crate::events::emit_connection_status(
                                        &bg_app,
                                        crate::events::ConnectionStatusPayload {
                                            online: false,
                                            offline_mode_manual: offline_manual,
                                            effective_offline: true,
                                        },
                                    );
                                });
                    }
                }
            }

            let auto_sync_settings = state.settings.clone();
            let auto_sync_engine = state.sync_engine.clone();
            let auto_sync_flag = state.sync_in_progress.clone();

            // macOS: opt the borderless main NSWindow into native fullscreen.
            // With `decorations: false` the window is created with a borderless
            // style mask and `collectionBehavior = default`, which does NOT
            // include `.fullScreenPrimary` — so macOS refuses fullscreen entirely
            // (green button no-op, View > Enter Full Screen disabled, moving
            // between Spaces ignored). Setting the flag once the window exists
            // makes `setFullscreen(true)` from JS work.
            //
            // Raw objc2 `msg_send!` avoids objc2-app-kit's feature-flag gating.
            // Only two calls are needed and the value of
            // NSWindowCollectionBehaviorFullScreenPrimary (1 << 7 = 128) is
            // stable Apple API.
            #[cfg(target_os = "macos")]
            {
                use objc2::msg_send;
                use objc2::runtime::AnyObject;

                // NSWindowCollectionBehaviorFullScreenPrimary from AppKit.
                const FULL_SCREEN_PRIMARY: usize = 1 << 7;

                if let Some(window) = app.get_webview_window("main") {
                    if let Ok(ns_window_ptr) = window.ns_window() {
                        let ns_window = ns_window_ptr as *mut AnyObject;
                        if !ns_window.is_null() {
                            unsafe {
                                let current: usize = msg_send![ns_window, collectionBehavior];
                                let _: () = msg_send![
                                    ns_window,
                                    setCollectionBehavior: current | FULL_SCREEN_PRIMARY
                                ];
                            }
                        }
                    }
                }
            }

            // OS media controls (Now Playing overlay + media keys). Desktop
            // registers MPRemoteCommandCenter / SMTC / MPRIS listeners via
            // souvlaki; iOS subscribes to events emitted by the Swift plugin
            // and pushes metadata through `MPNowPlayingInfoCenter` via the
            // plugin's `nowPlayingUpdate` method; Android is a no-op stub
            // pending a Media3/MediaSession integration.
            #[cfg(desktop)]
            let mc_result = crate::media_controls::create_media_controls(
                #[cfg(target_os = "windows")]
                &app.get_webview_window("main").expect("main window must exist"),
                player.clone(),
                image_cache_arc,
                client.clone(),
                http_client,
            );
            #[cfg(target_os = "ios")]
            let mc_result = crate::media_controls::create_media_controls(
                app_handle.clone(),
                player.clone(),
                image_cache_arc,
                client.clone(),
                http_client,
            );
            #[cfg(target_os = "android")]
            let mc_result = crate::media_controls::create_media_controls(
                app_handle.clone(),
                player.clone(),
                image_cache_arc,
                client.clone(),
                http_client,
            );
            match mc_result {
                Ok(handle) => {
                    *media_controls_ref.lock() = Some(handle);
                    log::info!("media controls initialized");
                }
                Err(e) => {
                    log::warn!("media controls unavailable: {e}");
                }
            }

            app.manage(state);

            session_reporter.ensure_loop_spawned();

            crate::auto_sync::spawn(auto_sync_settings, auto_sync_engine, auto_sync_flag);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // auth
            commands::auth::start_oauth,
            commands::auth::poll_oauth,
            commands::auth::discover_servers,
            commands::auth::test_server,
            commands::auth::connect_manual_url,
            commands::auth::find_music_libraries,
            commands::auth::finalize_onboarding,
            commands::auth::is_authenticated,
            commands::auth::logout,
            // library
            commands::library::get_genre_tree,
            commands::library::get_albums_for_genre,
            commands::library::get_albums_for_genre_names,
            commands::library::get_all_albums,
            commands::library::get_favourite_albums,
            commands::library::get_favourite_tracks,
            commands::library::get_albums_for_artist,
            commands::library::get_albums_for_artist_name,
            commands::library::get_albums_for_year,
            commands::library::get_tracks_for_album,
            commands::library::get_track,
            commands::library::get_all_artists,
            commands::library::get_favourite_genre_tree,
            commands::library::toggle_album_favourite,
            commands::library::toggle_track_favourite,
            commands::library::get_album_genres,
            commands::library::get_album,
            commands::library::get_random_album,
            commands::library::get_art_url,
            commands::library::get_album_colors,
            commands::library::set_album_palette,
            commands::library::get_cache_stats,
            // playback
            commands::playback::play_tracks,
            commands::playback::toggle_play_pause,
            commands::playback::next_track,
            commands::playback::previous_track,
            commands::playback::seek,
            commands::playback::set_volume,
            commands::playback::get_volume,
            commands::playback::append_to_queue,
            commands::playback::insert_next,
            commands::playback::remove_from_queue,
            commands::playback::jump_to_queue_index,
            commands::playback::get_queue,
            commands::playback::apply_equalizer,
            commands::playback::fetch_lyrics,
            commands::playback::get_waveform,
            // spectrum (focus-mode visualiser)
            commands::spectrum::get_spectrum,
            // search
            commands::search::search,
            commands::search::search_albums_for_grid,
            // sync
            commands::sync::start_full_sync,
            commands::sync::start_incremental_sync,
            commands::sync::start_genre_sync,
            // settings
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::import_custom_genres,
            commands::settings::remove_custom_genres,
            commands::settings::has_custom_genres,
            commands::settings::flush_image_cache,
            commands::settings::get_image_cache_stats,
            commands::settings::clear_audio_cache,
            commands::settings::get_audio_cache_stats,
            // platform
            commands::platform::dismiss_keyboard,
            commands::platform::show_native_search_bar,
            commands::platform::hide_native_search_bar,
            // acknowledgements / licenses
            commands::acknowledgements::get_acknowledgements_text,
            // downloads
            commands::downloads::download_track,
            commands::downloads::download_album,
            commands::downloads::download_all_starred_tracks,
            commands::downloads::download_all_starred_albums,
            commands::downloads::cancel_download,
            commands::downloads::cancel_all_downloads,
            commands::downloads::remove_download,
            commands::downloads::remove_album_downloads,
            commands::downloads::remove_all_downloads,
            commands::downloads::get_downloads_overview,
            commands::downloads::estimate_starred_tracks_size,
            commands::downloads::estimate_starred_albums_size,
            commands::downloads::download_search_results,
            commands::downloads::estimate_search_size,
            commands::downloads::get_connection_status,
        ])
        .build(tauri::generate_context!())
        .expect("error building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.session_reporter.stop_sync();
                    // Clear Now Playing so the OS widget disappears on quit.
                    if let Some(ref mc) = *state.media_controls.lock() {
                        mc.clear();
                    }
                }
            }
        });
}
