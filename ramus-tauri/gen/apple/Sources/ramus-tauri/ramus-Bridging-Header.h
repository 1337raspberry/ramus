#pragma once

// C ABI exposed by the Rust static library (libapp.a) for the App Intents /
// Siri integration. Implemented in ramus-tauri/src/siri_ffi.rs. Declared as
// plain C (no C++ namespace) so it can be imported through the Swift bridging
// header. Every returned string is heap-allocated by Rust and must be released
// with ramus_siri_free.

// Read-only: answer a library probe (genre nullable → no filter).
char *ramus_siri_probe(const char *genre);
// Read-only: list in-library genres / artists / albums (query nullable →
// suggestions; non-null → case-insensitive search, exact match preferred). JSON
// {ok, items, error}; album items carry {sourceId, title, artist}.
char *ramus_siri_genres(const char *query);
char *ramus_siri_artists(const char *query);
char *ramus_siri_albums(const char *query);
// Playback: start a genre, artist, or album (each arg nullable; album is a
// source id / rating key, not a title). Reaches the live player, so the app must
// be foregrounded. JSON {ok, spoken, trackCount}.
char *ramus_siri_play(const char *genre, const char *artist, const char *album);

void ramus_siri_free(char *ptr);

// iOS scene bridge: returns the main webview's UIWindow (as an ObjC id /
// UIWindow*), or NULL until it exists. The scene delegate adopts this window
// into the connected UIWindowScene — the windowing layer creates it without
// one, which is a black screen under the mandatory scene lifecycle. Unretained;
// main thread only. Implemented in siri_ffi.rs.
void *ramus_ios_main_window(void);
