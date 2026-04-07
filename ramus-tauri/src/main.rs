#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use ramus_tauri::commands;
use ramus_tauri::create_app_state;

fn main() {
    tauri::Builder::default()
        .manage(create_app_state())
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
            commands::library::get_all_albums,
            commands::library::get_favourite_albums,
            commands::library::get_albums_for_artist,
            commands::library::get_tracks_for_album,
            commands::library::get_all_artists,
            commands::library::get_favourite_genre_tree,
            commands::library::toggle_album_favourite,
            commands::library::toggle_track_favourite,
            commands::library::get_album_genres,
            commands::library::get_random_album,
            commands::library::get_art_url,
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
            // search
            commands::search::search,
            // sync
            commands::sync::start_full_sync,
            commands::sync::start_incremental_sync,
            commands::sync::start_genre_sync,
            // settings
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::import_custom_genres,
            commands::settings::remove_custom_genres,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
