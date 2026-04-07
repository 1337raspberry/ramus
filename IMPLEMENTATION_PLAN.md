# ramus Cross-Platform Rewrite: Implementation Plan

> **Rust + Tauri + React (TypeScript)**
> Port of the Swift/SwiftUI macOS music player to a cross-platform desktop app.
> This document is self-contained — all details needed to implement are here.

---

## Table of Contents

1. [Project Structure](#1-project-structure)
2. [Dependency Manifest](#2-dependency-manifest)
3. [Phase 1: Rust Core — Models & Types](#3-phase-1-rust-core--models--types)
4. [Phase 2: Rust Core — Plex API Client](#4-phase-2-rust-core--plex-api-client)
5. [Phase 3: Rust Core — Token Store & Auth](#5-phase-3-rust-core--token-store--auth)
6. [Phase 4: Rust Core — SQLite Cache](#6-phase-4-rust-core--sqlite-cache)
7. [Phase 5: Rust Core — Sync Engine](#7-phase-5-rust-core--sync-engine)
8. [Phase 6: Rust Core — Search & Query Parser](#8-phase-6-rust-core--search--query-parser)
9. [Phase 7: Rust Core — Genre Tree](#9-phase-7-rust-core--genre-tree)
10. [Phase 8: Rust Core — mpv Playback](#10-phase-8-rust-core--mpv-playback)
11. [Phase 9: Rust Core — Audio Player & Queue](#11-phase-9-rust-core--audio-player--queue)
12. [Phase 10: Rust Core — Lyrics, Waveform, Session Reporting](#12-phase-10-rust-core--lyrics-waveform-session-reporting)
13. [Phase 11: Rust Core — Connection Monitor & Media Keys](#13-phase-11-rust-core--connection-monitor--media-keys)
14. [Phase 12: Tauri Shell & IPC](#14-phase-12-tauri-shell--ipc)
15. [Phase 13: React Frontend — Layout & Navigation](#15-phase-13-react-frontend--layout--navigation)
16. [Phase 14: React Frontend — Genre Tree & Album Grid](#16-phase-14-react-frontend--genre-tree--album-grid)
17. [Phase 15: React Frontend — Now Playing, Waveform, Lyrics](#17-phase-15-react-frontend--now-playing-waveform-lyrics)
18. [Phase 16: React Frontend — Search, EQ, Queue, Settings](#18-phase-16-react-frontend--search-eq-queue-settings)
19. [Phase 17: React Frontend — Onboarding](#19-phase-17-react-frontend--onboarding)
20. [Phase 18: Polish, Platform Packaging, Testing](#20-phase-18-polish-platform-packaging-testing)
21. [Appendix A: Full Database Schema](#appendix-a-full-database-schema)
22. [Appendix B: Plex API Endpoint Reference](#appendix-b-plex-api-endpoint-reference)
23. [Appendix C: mpv Property & Command Reference](#appendix-c-mpv-property--command-reference)
24. [Appendix D: Test Specification](#appendix-d-test-specification)
25. [Appendix E: Platform-Specific Concerns](#appendix-e-platform-specific-concerns)

---

## 1. Project Structure

```
ramus/
├── Cargo.toml                       # Workspace root
├── IMPLEMENTATION_PLAN.md           # This file
├── CLAUDE.md                        # AI assistant instructions
├── ramus-core/                      # Rust library crate (all business logic)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                   # Public API re-exports
│       ├── models.rs                # Album, Track, PlexServer, PlayerState, etc.
│       ├── plex/
│       │   ├── mod.rs
│       │   ├── client.rs            # HTTP client, all Plex endpoints
│       │   ├── auth.rs              # PIN-based OAuth, token persistence
│       │   ├── token_store.rs       # AES-GCM encrypted file storage
│       │   └── connection.rs        # Connection monitoring, failover
│       ├── playback/
│       │   ├── mod.rs
│       │   ├── mpv.rs               # libmpv C API wrapper
│       │   ├── player.rs            # AudioPlayer — queue, cache, gapless
│       │   ├── transcode.rs         # Codec check, HLS vs direct play URLs
│       │   ├── waveform.rs          # dB → normalised amplitudes
│       │   ├── lyrics.rs            # LRC parsing, LRCLIB HTTP, Plex lyrics
│       │   ├── session.rs           # Timeline reporting, scrobble
│       │   └── media_keys.rs        # OS media key integration (souvlaki)
│       ├── cache/
│       │   ├── mod.rs
│       │   ├── db.rs                # rusqlite DatabasePool, migrations, CRUD
│       │   └── sync.rs              # Plex → SQLite incremental sync
│       ├── search/
│       │   ├── mod.rs
│       │   ├── parser.rs            # Operator syntax (/ @ $ % ! year: rating:)
│       │   └── engine.rs            # FTS5 + fuzzy hybrid search
│       └── genre/
│           ├── mod.rs
│           ├── node.rs              # GenreNode tree model
│           ├── mapper.rs            # Flat→hierarchy mapping, fuzzy cache
│           └── parser.rs            # Custom indented text → genre JSON
├── ramus-tauri/                     # Tauri app shell
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   ├── build.rs
│   └── src/
│       ├── main.rs                  # Tauri entry point
│       ├── commands/                # #[tauri::command] handlers
│       │   ├── mod.rs
│       │   ├── library.rs           # Genre/album/track navigation
│       │   ├── playback.rs          # Transport, queue, EQ
│       │   ├── search.rs            # Query + results
│       │   ├── sync.rs              # Sync triggers + progress
│       │   ├── auth.rs              # OAuth flow, server config
│       │   └── settings.rs          # Preferences read/write
│       ├── state.rs                 # Tauri managed state (AppState)
│       └── events.rs                # Event names + payload types
├── ui/                              # React frontend (Vite + TypeScript)
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── src/
│   │   ├── main.tsx                 # React entry point
│   │   ├── App.tsx                  # Root component, routing
│   │   ├── types/                   # TypeScript type definitions
│   │   │   ├── models.ts            # Album, Track, GenreNode, etc.
│   │   │   ├── events.ts            # Tauri event payloads
│   │   │   └── commands.ts          # Tauri command return types
│   │   ├── hooks/                   # Custom React hooks
│   │   │   ├── usePlayback.ts       # Player state, transport controls
│   │   │   ├── useLibrary.ts        # Genre/album/track navigation
│   │   │   ├── useSearch.ts         # Search state + debounce
│   │   │   ├── useSettings.ts       # Preferences
│   │   │   └── useTauriEvent.ts     # Generic Tauri event listener
│   │   ├── components/
│   │   │   ├── layout/
│   │   │   │   └── ThreeColumnLayout.tsx
│   │   │   ├── sidebar/
│   │   │   │   ├── SidebarView.tsx
│   │   │   │   └── GenreTreeView.tsx
│   │   │   ├── content/
│   │   │   │   ├── AlbumGridView.tsx
│   │   │   │   └── TrackListView.tsx
│   │   │   ├── detail/
│   │   │   │   ├── NowPlayingView.tsx
│   │   │   │   ├── LyricsView.tsx
│   │   │   │   ├── QueueView.tsx
│   │   │   │   └── SuggestedAlbumView.tsx
│   │   │   ├── search/
│   │   │   │   └── SearchOverlay.tsx
│   │   │   ├── onboarding/
│   │   │   │   ├── OnboardingFlow.tsx
│   │   │   │   ├── OAuthSignIn.tsx
│   │   │   │   ├── ServerPicker.tsx
│   │   │   │   ├── LibraryPicker.tsx
│   │   │   │   └── InitialSync.tsx
│   │   │   └── shared/
│   │   │       ├── WaveformSeekBar.tsx
│   │   │       ├── EqualizerPanel.tsx
│   │   │       ├── FlowLayout.tsx
│   │   │       └── VolumeSlider.tsx
│   │   ├── stores/                  # Zustand stores (or similar)
│   │   │   ├── playbackStore.ts
│   │   │   ├── libraryStore.ts
│   │   │   └── settingsStore.ts
│   │   └── styles/
│   │       ├── global.css
│   │       └── theme.ts            # CSS variable system for accent colors
│   └── public/
│       └── open.json               # Wikidata genre hierarchy (copied from Swift project)
└── scripts/
    └── build_open_genres.py         # Wikidata SPARQL → open.json generator
```

---

## 2. Dependency Manifest

### Rust (`ramus-core/Cargo.toml`)

```toml
[package]
name = "ramus-core"
version = "0.1.0"
edition = "2021"

[dependencies]
# HTTP
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
# JSON
serde = { version = "1", features = ["derive"] }
serde_json = "1"
# SQLite
rusqlite = { version = "0.31", features = ["bundled", "column_decltype"] }
# Crypto
aes-gcm = "0.10"
sha2 = "0.10"
# mpv
libmpv2 = "4"          # Safe Rust bindings for libmpv (evaluate; fallback: raw libmpv-sys FFI)
# Fuzzy search
fuzzy-matcher = "0.3"
# Media keys
souvlaki = "0.7"
# Utilities
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4"] }
log = "0.4"
env_logger = "0.11"
parking_lot = "0.12"   # Fast Mutex/RwLock
directories = "5"       # Platform XDG/AppData paths
thiserror = "2"
url = "2"

[target.'cfg(target_os = "macos")'.dependencies]
core-foundation = "0.10"    # IOKit for hardware UUID

[target.'cfg(target_os = "windows")'.dependencies]
winreg = "0.52"             # Registry for MachineGuid

[target.'cfg(target_os = "linux")'.dependencies]
# /etc/machine-id — just std::fs::read_to_string

[dev-dependencies]
tokio = { version = "1", features = ["test-util", "macros"] }
tempfile = "3"
```

> **Note on libmpv crate**: Evaluate `libmpv2` crate for coverage of needed commands.
> If it lacks playlist management or `af` filter support, use raw `libmpv-sys` FFI
> (closer to the Swift implementation). The Swift `MPVController` is ~440 lines of
> raw C API calls — a direct `libmpv-sys` port may be simpler than adapting a
> high-level wrapper.

### Tauri (`ramus-tauri/Cargo.toml`)

```toml
[package]
name = "ramus-tauri"
version = "0.1.0"
edition = "2021"

[dependencies]
ramus-core = { path = "../ramus-core" }
tauri = { version = "2", features = ["tray-icon", "devtools"] }
tauri-plugin-shell = "2"      # Open URLs in browser (OAuth)
tauri-plugin-dialog = "2"     # File picker (custom genres)
tauri-plugin-fs = "2"         # File system access
tauri-plugin-process = "2"    # App lifecycle
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
log = "0.4"

[build-dependencies]
tauri-build = "2"
```

### React (`ui/package.json`)

```json
{
  "name": "ramus-ui",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-shell": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "@tauri-apps/plugin-fs": "^2",
    "@tanstack/react-virtual": "^3",
    "zustand": "^5",
    "react": "^19",
    "react-dom": "^19"
  },
  "devDependencies": {
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "@vitejs/plugin-react": "^4",
    "typescript": "^5.6",
    "vite": "^6"
  }
}
```

---

## 3. Phase 1: Rust Core — Models & Types

**Goal:** Define all shared data types. No I/O, no dependencies beyond `serde`.

### File: `ramus-core/src/models.rs`

#### Type Aliases
```rust
pub type PlexID = String;
pub type Duration = f64; // seconds (TimeInterval equivalent)
```

#### Enums

**RangeOp** — for search year:/rating: filters:
```
Equal, GreaterThan, LessThan, GreaterOrEqual, LessOrEqual
```
Each has a `sql_literal(&self) -> &str` method returning `"="`, `">"`, `"<"`, `">="`, `"<="`.

**RangeField** — `Year`, `Rating`

**PlaybackStatus** — `Playing`, `Paused`, `Stopped`

**PlaybackMode** — `DirectPlay`, `TranscodeLosslessRemote`, `TranscodeLossless`

**SearchResultKind** — `Album`, `Track`

#### Structs (all `Serialize, Deserialize, Clone, Debug`)

**Album:**
- `rating_key: PlexID` (primary key, used as `id`)
- `title: String`
- `artist_name: String`
- `year: Option<i32>`
- `thumb: Option<String>`
- `genres: Vec<String>`
- `is_favourite: bool`
- `studio: Option<String>`
- `added_at: Option<i64>` (Unix timestamp)
- `last_viewed_at: Option<i64>`

**Track:**
- `rating_key: PlexID`
- `title: String`
- `artist_name: String` (album artist)
- `track_artist: Option<String>` (per-track override)
- `album_title: String`
- `album_key: Option<PlexID>`
- `index: Option<i32>` (track number)
- `duration: Duration` (seconds)
- `codec: Option<String>`
- `part_key: Option<String>`
- `thumb: Option<String>`
- `is_favourite: bool`
- `bitrate: Option<i32>`
- `disc_number: Option<i32>`
- Methods:
  - `display_artist(&self) -> &str` — `track_artist` if present/non-empty/different from `artist_name`, else `artist_name`
  - `has_track_artist(&self) -> bool` — true if track artist differs (case-insensitive)
  - `format_description(&self) -> Option<String>` — "FLAC" for lossless (`flac|alac|wav|aiff|pcm`), "MP3 320 kbps" for lossy with bitrate

**UltraBlurColors:**
- `top_left: String`, `top_right: String`, `bottom_right: String`, `bottom_left: String`

**PlexServerConnection:**
- `uri: String`, `local: bool`, `relay: bool`, `protocol: String`
- `priority(&self) -> u8` — 0=local+HTTPS, 1=remote+HTTPS, 2=relay+HTTPS, 3=local+HTTP, 4=remote+HTTP, 5=relay+HTTP

**PlexServer:**
- `machine_identifier: String` (used as `id`)
- `name: String`
- `access_token: String`
- `owned: bool`
- `connections: Vec<PlexServerConnection>`
- `sorted_connections(&self) -> Vec<&PlexServerConnection>` — sorted by priority ascending

**ServerConfig:**
- `machine_identifier: String`
- `name: String`
- `access_token: String` (excluded from JSON serialization — stored in token store)
- `selected_library_key: Option<String>`
- Custom Serialize: skip `access_token`. Custom Deserialize: default `access_token` to empty string.

**PlayerState:**
- `status: PlaybackStatus`
- `current_track: Option<Track>`
- `queue: Vec<Track>`
- `queue_index: usize`

**PlaybackConfig:**
- `playback_mode: PlaybackMode`
- `lookahead_depth: u8` (clamped 1–20, default 3)
- `audio_cache_limit_bytes: i64` (default 2_147_483_648)

**SearchResult:**
- `id: String` — `"album-{source_id}"` or `"track-{id}"`
- `kind: SearchResultKind`
- `album_source_id: String`
- `album_title: String`, `artist_name: String`, `year: Option<i32>`
- `album_art_path: Option<String>`
- `track_source_id: Option<String>`, `track_title: Option<String>`, `track_artist: Option<String>`
- `score: f64` (0.0=exact, higher=worse)

**LibrarySection:**
- `key: String`, `title: String`, `section_type: String`

### Verification
```
cargo test -p ramus-core
```
Write unit tests for:
- `Track::display_artist` and `has_track_artist` logic
- `Track::format_description` for various codecs
- `PlexServerConnection::priority` ordering
- `PlaybackConfig` clamping
- `ServerConfig` serialisation (access_token excluded)

---

## 4. Phase 2: Rust Core — Plex API Client

**Goal:** HTTP client for all Plex endpoints. Uses `reqwest`.

### File: `ramus-core/src/plex/client.rs`

#### State
- `client_identifier: String` — persistent UUID (loaded from settings file, generated on first run)
- `server_url: Option<Url>` — current connection base URL
- `token: Option<String>` — current access token
- `on_request_failed: Option<Box<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>>` — reconnection callback
- Thread safety: `parking_lot::RwLock` around mutable fields (or `tokio::sync::RwLock` for async)

#### Standard Headers (applied to every request)
```
Accept: application/json
X-Plex-Client-Identifier: {client_identifier}
X-Plex-Product: ramus
X-Plex-Platform: macOS | Windows | Linux   (compile-time cfg)
X-Plex-Device: Mac | PC | Linux            (compile-time cfg)
X-Plex-Token: {token}                      (if set)
```

#### Endpoints to Implement

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `discover_servers(auth_token)` | GET `https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1` | Server discovery. Filter for `provides: "server"` with non-null `accessToken`. Max 50MB response. |
| `test_connection(uri, token, timeout)` | GET `{uri}/identity` | Returns bool. Default 5s timeout. |
| `find_best_connection(server, allow_http)` | Tests all connections concurrently | Returns highest-priority working connection. Log warning if HTTP. |
| `connect(server_url, token)` | GET `{server_url}/identity` | Verify connectivity. Throw on non-2xx. |
| `find_music_libraries()` | GET `library/sections` | Filter for `type == "artist"`. Error if none. |
| `fetch_all_items(library_key, type, page_size)` | GET `library/sections/{key}/all?type={t}&X-Plex-Container-Start={offset}&X-Plex-Container-Size={size}` | Paginated fetch. Type codes: 8=artist, 9=album, 10=track. Page size 200. Max 5000 pages. |
| `fetch_item_metadata(rating_key)` | GET `library/metadata/{rating_key}` | Full metadata (all genres, streams, colors). |
| `fetch_lyrics_stream(rating_key)` | Via `fetch_item_metadata` → find stream with `stream_type == 4` | Lyrics stream info. |
| `download_lyrics_data(path)` | GET `{path}` with `Accept: */*` | Raw LRC/TXT bytes. |
| `fetch_levels(stream_id, subsample)` | GET `library/streams/{stream_id}/levels?subsample={n}` | Loudness data. Default subsample=600. |
| `report_timeline(rating_key, state, time_ms, duration_ms, session_id)` | PUT `/:/timeline?ratingKey={rk}&key=/library/metadata/{rk}&state={s}&time={t}&duration={d}&identifier=com.plexapp.plugins.library&X-Plex-Token={token}` | Header: `X-Plex-Session-Identifier`. Fire-and-forget. |
| `scrobble(rating_key)` | PUT `/:/scrobble?key=/library/metadata/{rk}&identifier=com.plexapp.plugins.library&X-Plex-Token={token}` | Fire-and-forget. |
| `rate_item(rating_key, rating)` | PUT `:/rate?key={rk}&identifier=com.plexapp.plugins.library&rating={r}` | rating: 10.0=favourite, 0.0=unfavourite. |

#### Plex API Response Models (internal, for JSON deserialization)

**MediaItem** (the universal Plex metadata object):
- `rating_key, title, title_sort, original_title, summary`
- `parent_title, grandparent_title, parent_rating_key, grandparent_rating_key`
- `index, parent_index` (track number, disc number)
- `year, duration` (ms!), `updated_at, added_at, last_viewed_at` (Unix seconds)
- `thumb, parent_thumb, grandparent_thumb, art`
- `user_rating: Option<f64>` (0–10)
- `studio: Option<String>`
- `media: Option<Vec<MediaInfo>>`, `genre: Option<Vec<PlexTag>>`
- `ultra_blur_colors: Option<UltraBlurColors>`

**MediaInfo:** `audio_codec, bitrate, parts: Vec<PartInfo>`
**PartInfo:** `key, streams: Vec<StreamInfo>`
**StreamInfo:** `id, stream_type, codec, bitrate, key, format, timed: Option<bool>, provider: Option<String>`
  - Custom deserialize: `timed` can be JSON bool OR int (convert int→bool)

**PlexTag:** `tag: String`
**LevelSample:** `v: f32`

#### Error Handling
- Connection errors (dns, timeout, connection lost) → call `on_request_failed`, retry once if it returns true
- HTTP 401 → `Unauthorized` error
- Non-2xx → `HttpError(status_code)`
- JSON decode failure → `InvalidResponse`

#### Error Enum: `PlexClientError`
`NotConnected`, `ConnectionFailed`, `NoMusicLibrary`, `InvalidResponse`, `Unauthorized`, `HttpError(u16)`, `NoSecureConnection`

### Verification
- Unit tests with mock HTTP (use `wiremock` or `mockito` crate)
- Test header construction
- Test pagination loop logic
- Test error mapping (401→Unauthorized, connection errors→retry)

---

## 5. Phase 3: Rust Core — Token Store & Auth

### File: `ramus-core/src/plex/token_store.rs`

#### Storage
- **Directory:** platform config dir / `ramus/` (use `directories` crate: `ProjectDirs::from("com", "raspsoft", "ramus")`)
  - macOS: `~/Library/Application Support/ramus/`
  - Linux: `~/.local/share/ramus/`
  - Windows: `%APPDATA%\raspsoft\ramus\`
- **File:** `tokens.enc`
- **Permissions:** 0o600 (Unix), default ACL (Windows)

#### Encryption
- **Algorithm:** AES-256-GCM
- **Key derivation:** SHA-256 of machine UUID string → 32-byte key
- **Machine UUID source:**
  - macOS: `IOPlatformUUID` via `IOKit` (use `core-foundation` + raw `IOServiceMatching`)
  - Windows: `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` (via `winreg`)
  - Linux: `/etc/machine-id` (trim whitespace)
- **Nonce:** random 12 bytes per seal (prepended to ciphertext)
- **Format:** encrypted blob = nonce (12 bytes) || ciphertext || tag (16 bytes)

#### Storage Format
- Plaintext is JSON: `{"plexAuthToken": "...", "plexServerToken": "..."}`
- Read all → decrypt → parse JSON → return requested key
- Write: read existing (or empty dict) → insert/update key → encrypt → atomic write

#### Token Keys
- `AuthToken = "plexAuthToken"` — main plex.tv auth token
- `ServerToken = "plexServerToken"` — per-server access token

#### Public API
```rust
pub fn read(key: TokenKey) -> Option<String>
pub fn write(key: TokenKey, value: &str) -> bool
pub fn delete(key: TokenKey) -> bool
```
All operations: lock (Mutex), decrypt file, operate, re-encrypt, write. Thread-safe.

### File: `ramus-core/src/plex/auth.rs`

#### PIN-Based OAuth Flow

1. **Create PIN:** POST `https://plex.tv/api/v2/pins?strong=true&X-Plex-Product=ramus&X-Plex-Client-Identifier={id}`
   - Response: `PinResponse { id: i64, code: String, auth_token: Option<String> }`

2. **Build auth URL:** `https://app.plex.tv/auth#?clientID={id}&code={code}&context%5Bdevice%5D%5Bproduct%5D=ramus`
   - Open in system browser (via Tauri shell plugin)

3. **Poll for token:** GET `https://plex.tv/api/v2/pins/{pin_id}` with headers
   - Poll every 2 seconds, max 60 attempts (~2 min)
   - Return when `auth_token` is populated
   - HTTP 404 → pin expired
   - Max attempts → timeout error

#### Server Config Persistence
- `store_server_config(config)` — token → encrypted token store, redacted config → JSON file in config dir
- `stored_server_config() -> Option<ServerConfig>` — read JSON + reconstitute token from store
- `delete_server_config()` — remove both

#### Errors
`PinCreationFailed`, `PollingTimeout`, `PinExpired`

### Verification
- Test token store round-trip (write → read → delete)
- Test encryption (different machine IDs produce different ciphertext)
- Test PIN response parsing
- Test auth URL construction

---

## 6. Phase 4: Rust Core — SQLite Cache

**Goal:** Full GRDB-equivalent cache with rusqlite. WAL mode, FTS5.

### File: `ramus-core/src/cache/db.rs`

#### Database Location
- `{data_dir}/ramus/cache.db` (same `directories` crate path as token store)

#### Configuration
```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
```

#### Migration: `v1_complete`

See [Appendix A](#appendix-a-full-database-schema) for complete schema. Summary:

| Table | Purpose | Key columns |
|-------|---------|-------------|
| `artists` | Artist metadata | `source_id UNIQUE`, name, sort_name, art_url, updated_at |
| `albums` | Album metadata | `source_id UNIQUE`, title, artist_id FK, year, art_url, rating, studio, ultra_blur_colors (JSON), added_at, last_viewed_at, updated_at |
| `tracks` | Track metadata | `source_id UNIQUE`, title, album_id FK, artist_id FK, track_number, disc_number, duration_ms, codec, part_key, stream_id, user_rating, bitrate, track_artist, updated_at |
| `tracks_fts` | FTS5 virtual table | content=tracks, title column, unicode61 tokenizer, prefixes=2,3 |
| `genres` | Genre names | `name UNIQUE COLLATE NOCASE` |
| `album_genres` | Album↔Genre links | `(album_id, genre_id) PK` |

#### CRUD Operations

**Batch Upserts (transactional, batches of 500):**
- `batch_upsert_artists(items) -> HashMap<String, i64>` — sourceId → local id
- `batch_upsert_albums(items) -> HashMap<String, i64>` — COALESCE preserves existing rating/studio/colors
- `batch_upsert_tracks(items)` — includes FTS5 trigger rebuild
- `batch_upsert_genres_and_links(items)` — upsert genre name + album↔genre link

**Timestamp Lookups (for incremental sync):**
- `all_artist_timestamps() -> HashMap<String, CachedItemInfo>` — sourceId → (id, updated_at)
- `all_album_timestamps() -> HashMap<String, CachedAlbumInfo>` — includes first genre name (subquery)
- `all_track_timestamps() -> HashMap<String, CachedItemInfo>`

**Genre Management:**
- `upsert_genre(name) -> i64` — case-insensitive, returns id
- `link_album_genre(album_id, genre_id)` — INSERT OR IGNORE
- `set_album_genres(album_id, genre_ids)` — delete old + insert new
- `update_album_deep_metadata(album_id, genre_ids, rating, studio, colors_json)` — atomic

**ID Lookups:**
- `artist_id(source_id) -> Option<i64>`
- `album_id(source_id) -> Option<i64>`
- `album_updated_at(source_id) -> Option<i64>`

**Query Methods:**
- `albums_for_genre(genre_name) -> Vec<Album>`
- `albums_for_artist(source_id) -> Vec<Album>`
- `tracks_for_album(source_id) -> Vec<Track>`
- `favourite_albums() -> Vec<Album>`
- `all_albums() -> Vec<Album>`
- `all_artists() -> Vec<(i64, String, String, Option<String>)>` — (id, name, sourceId, artUrl)
- `genre_album_sets() -> HashMap<String, HashSet<i64>>` — genre name → set of album IDs
- `search_tracks_fts(query) -> Vec<Track>` — FTS5 prefix search
- `search_albums_by_title(query) -> Vec<Album>`
- `albums_by_year_range(op, value) -> HashSet<i64>`
- `albums_by_rating_range(op, value) -> HashSet<i64>`
- `cache_stats() -> CacheStats` — counts of artists, albums, tracks, genres
- `update_album_rating(source_id, rating)` — for favourite toggle
- `update_track_rating(source_id, rating)` — for favourite toggle
- `random_album() -> Option<Album>` — ORDER BY RANDOM() LIMIT 1
- `album_genres(source_id) -> Vec<String>` — genre names for display

#### FTS5 Sync
On track upsert, also update `tracks_fts`:
```sql
INSERT INTO tracks_fts(rowid, title) VALUES (?, ?)
ON CONFLICT DO UPDATE SET title = excluded.title;
```
Or use triggers (INSERT/UPDATE/DELETE on tracks → mirror to tracks_fts).

#### Connection Pool
Use `rusqlite::Connection` with a `Mutex` (rusqlite connections are not `Send`).
For read concurrency, consider `r2d2-sqlite` or a custom pool.
Alternatively: single writer + multiple reader connections (WAL mode supports this).

### Verification
- Test all CRUD operations (insert, update, upsert conflict handling)
- Test FTS5 prefix search (`"par"*` matches "Paranoid Android")
- Test genre upsert case-insensitivity
- Test batch operations with 500+ items
- Test timestamp queries return correct maps
- Test LIKE pattern escaping (`%`, `_`, `\` in search terms)
- Test year range filters (=, >, <, >=, <=, null excluded)

---

## 7. Phase 5: Rust Core — Sync Engine

### File: `ramus-core/src/cache/sync.rs`

#### Three Sync Modes

| Mode | When | Artists/Albums/Tracks | Deep Genre Fetch |
|------|------|----------------------|-----------------|
| `full_sync` | Initial setup, library reset | All | All albums |
| `incremental_sync` | Periodic (most common) | Changed only | Changed/new albums only |
| `genre_sync` | Genre edits on server | Skip | All albums |

#### Sync Algorithm (4 phases)

**Phase 1: Artists (type=8)**
- Fetch all via `client.fetch_all_items(library_key, 8)`
- Compare `updated_at` against cached timestamps
- Batch upsert changed/new artists (batches of 500)
- Return `HashMap<String, i64>` (sourceId → local id)

**Phase 2: Albums (type=9)**
- Fetch all via `client.fetch_all_items(library_key, 9)`
- Change detection: `updated_at` differs OR first genre tag differs (genre-only edits don't always bump `updated_at`)
- Batch upsert changed/new albums
- Collect shallow genre links from `item.genre[0]` (list views return only 1 genre)
- Return `(HashMap<String, i64>, HashSet<String>)` — (id map, changed sourceIds)

**Phase 3: Tracks (type=10)**
- Fetch all via `client.fetch_all_items(library_key, 10)`
- Audio stream extraction: `item.media[0].parts[0].streams.find(stream_type == 2)`
- Codec priority: `stream.codec ?? media.audio_codec`
- Bitrate priority: `stream.bitrate ?? media.bitrate`
- Batch upsert changed tracks

**Phase 4: Deep Genre Sync**
- Bounded concurrency: 8 concurrent requests (`tokio::sync::Semaphore`)
- For each album in scope (all for full/genre sync, changed set for incremental):
  - `client.fetch_item_metadata(source_id)` → extract all genres + rating + studio + ultra_blur_colors
  - `db.update_album_deep_metadata(album_id, genre_ids, rating, studio, colors_json)`
- Skip individual album failures silently (deleted albums, 404s)

#### Progress Reporting
```rust
pub struct SyncProgress {
    pub phase: SyncPhase,     // Artists, Albums, Tracks, DeepGenres, Done
    pub current: usize,
    pub total: usize,
    pub detail: String,
}
impl SyncProgress {
    pub fn fraction(&self) -> f64 { if self.total > 0 { self.current as f64 / self.total as f64 } else { 0.0 } }
}
```
Progress callback: `Fn(SyncProgress) + Send + Sync`

#### Error Handling
- Re-throw `Unauthorized` and `NotConnected` (abort sync)
- Silently skip individual album failures in Phase 4
- Set `last_sync_time` in settings on completion

### Verification
- Test incremental sync skips unchanged items
- Test genre change detection (same `updated_at` but different first genre)
- Test batch size boundaries (exactly 500, 501 items)
- Test Phase 4 concurrent fetch (mock 8+ albums)
- Test progress callback fires with correct fractions

---

## 8. Phase 6: Rust Core — Search & Query Parser

### File: `ramus-core/src/search/parser.rs`

#### Operator Syntax

| Prefix | Meaning | Consumes |
|--------|---------|----------|
| `/` | Genre filter | All text until `AND` |
| `@` | Artist filter | All text until `AND` |
| `!` | Track search (sets `has_track_search`) | All text until `AND` |
| `%` | Album title filter | All text until `AND` |
| `fav:` or `favourites:` | Favourites-only (case-insensitive prefix) | Just the keyword |
| `$` or `year:` | Year range | Value after prefix (e.g., `>2000`) |
| `rating:` | Rating range | Value after prefix |
| (none) | Free text | All text until `AND` |

#### Parsing Rules
1. Split input on `" AND "` (case-sensitive, literal uppercase)
2. For each segment, detect operator prefix
3. Extract value (trim whitespace)
4. **Without AND, entire input belongs to first operator:** `/rock year:2000` → genre filter with value `"rock year:2000"`
5. Bare operators (`/`, `@`, `!`, `%` alone) are ignored
6. Invalid range values fall back to free text: `year:abc` → freeText(`"year:abc"`)

#### Range Parsing
- `>=` → GreaterOrEqual
- `<=` → LessOrEqual
- `>` → GreaterThan
- `<` → LessThan
- (none) → Equal
- Value must parse as `f64`, otherwise fall back to free text

#### FTS5 Escaping (`escape_fts5`)
- Strip: `"`, `*`, `(`, `)`, `:`, `^`, `{`, `}`
- Replace `-` with space (FTS5 NOT operator)
- Keep alphanumeric, spaces, Unicode

#### Output
```rust
pub struct ParsedQuery {
    pub filters: Vec<SearchFilter>,
}
// Convenience methods:
// free_text() -> Option<&str>
// genre_filters() -> Vec<&str>
// artist_filters() -> Vec<&str>
// album_title_filters() -> Vec<&str>
// track_searches() -> Vec<&str>
// range_filters() -> Vec<(RangeField, RangeOp, f64)>
// has_track_search() -> bool
// has_favourites_filter() -> bool
// is_free_text_only() -> bool
// is_empty() -> bool
```

### File: `ramus-core/src/search/engine.rs`

#### Search Logic

**`search(query: &str, limit: usize) -> Vec<SearchResult>`**
1. Parse query via `QueryParser::parse(query)`
2. Resolve album constraints (genre expansion, year/rating ranges, favourites) → `Option<HashSet<i64>>`
3. If `has_track_search`: return tracks only via `search_tracks`
4. If free text only: return albums (max 5) + tracks (max `limit`)
5. Otherwise: return albums matching filters

**Album Search:**
- Match by artist name (LIKE `%query%`) or album title
- Filter by resolved album ID set
- Score: exact=0.0, starts-with=0.02, contains=0.05

**Track Search (`search_tracks_by_text`):**
1. Escape query for FTS5
2. Split into tokens, wrap each: `"token"*`
3. Execute FTS5 query against `tracks_fts`
4. If < 5 results, run fuzzy fallback

**Fuzzy Fallback:**
- Use `fuzzy-matcher` crate (or `sublime_fuzzy`)
- Search composite string: `"{artist_name} {album_title} {track_title}"`
- Threshold: filter scores > 0.4 relative quality
- Prefix fuzzy scores with 0.5 (rank below FTS5)
- Limit to 50 fuzzy results

**Genre Expansion:**
- `/metal` → collect all descendant genre names from genre tree
- Query albums where any genre matches

### Verification

Port all 27 QueryParser tests + 13 SearchEngine tests from Swift:
- `test_parse_free_text`, `test_parse_genre_filter`, `test_parse_artist_filter`
- `test_parse_album_title_filter`, `test_parse_track_search`
- `test_parse_multi_word_track_search`, `test_parse_year_greater_than`
- `test_parse_combined_with_and`, `test_operator_without_and_consumes_all`
- `test_parse_empty_input`, `test_parse_invalid_year`, `test_parse_bare_operators`
- `test_escape_fts5`, `test_escape_fts5_hyphen_replaced_with_space`
- `test_free_text_search_returns_albums_and_tracks`
- `test_genre_filter_expands_hierarchy`
- `test_track_search_fuzzy_fallback` (typo `"paranoyd"` finds "Paranoid Android")
- `test_free_text_albums_capped_at_five`
- Full list in [Appendix D](#appendix-d-test-specification)

---

## 9. Phase 7: Rust Core — Genre Tree

### File: `ramus-core/src/genre/node.rs`

```rust
pub struct GenreNode {
    pub id: String,                      // path-based: "rock/metal/thrash metal"
    pub name: String,                    // display name (title-cased)
    pub short_summary: Option<String>,
    pub children: Option<Vec<GenreNode>>, // None for leaf nodes
    pub album_count: usize,
    pub deduplicated_total_count: usize,
}
```
- `all_descendant_names(&self) -> Vec<String>` — recursive collect
- `collect_descendant_names(&self, into: &mut HashSet<String>)` — efficient set variant

### File: `ramus-core/src/genre/mapper.rs`

#### Initialisation
- Load from JSON file (bundled `open.json` or custom file path)
- Recursively decode genre hierarchy
- Apply title-casing during load
- Build:
  1. `root_nodes: Vec<GenreNode>` — full hierarchy
  2. `exact_lookup: HashMap<String, GenreNode>` — lowercased name → node (clone/ref)
  3. `all_names: Vec<String>` — for fuzzy search
  4. Fuzzy matcher instance (threshold ~0.4)

#### Title-Casing Algorithm
```
For each word (split on spaces):
  For each segment (split on hyphens):
    If all lowercase → capitalize first letter
    If has any uppercase → leave unchanged (preserves "EBM", "R&B")
  Rejoin with hyphens
Rejoin with spaces
```
Examples: `"ambient music"` → `"Ambient Music"`, `"lo-fi"` → `"Lo-Fi"`, `"R&B"` → `"R&B"`

#### Genre Matching (with cache)
```
match_genre(plex_genre: &str) -> Option<GenreNode>
  1. Check hit cache → return clone
  2. Check miss cache → return None
  3. Exact match (lowercase) → cache + return
  4. Fuzzy search all_names → cache best match or miss
```
Cache protected by `parking_lot::Mutex`.

#### Display Tree Building
```
build_display_tree(genre_album_sets: HashMap<String, HashSet<i64>>) -> Vec<GenreNode>
  1. Prune branches with no albums (recursive)
  2. Compute deduplicated counts (post-order union of descendant album ID sets)
  3. Create "Other" node for unmatched genres
  4. Return pruned + decorated tree
```

### File: `ramus-core/src/genre/parser.rs`

#### Custom Genre Text Format
Indented plain text where depth = indentation level. Optional `[description]` brackets.

```
Rock [Guitar-based music]
  Alternative Rock
    Shoegaze [Wall of sound]
  Punk Rock
```

#### Parsing Steps
1. **Validate limits:** max 1MB file, max 50,000 lines, max 200 char genre names
2. **Sanitize:** strip C0 control chars (except tab), DEL, C1 control chars
3. **Reject JSON:** first non-whitespace `{` or `[` → error
4. **Detect indent unit:** first indented line → tab or N spaces (4+→4, 2-3→2, 1→1)
5. **Parse lines:** count indent depth, extract name + optional `[description]`
6. **Validate:** no indentation jumps > 1 level
7. **Detect duplicates:** per-parent, case-insensitive (warning, not error)
8. **Build tree:** stack-based depth-first construction
9. **Output:** JSON compatible with `GenreMapper` format

#### Errors
`EmptyFile`, `NotPlainText`, `FileTooLarge(usize)`, `TooManyLines(usize)`, `NameTooLong(usize, String)`, `UnmatchedBracket(usize)`, `IndentationJump(usize, usize, usize)`, `NoRootGenresFound`

### Verification
Port all 61 genre-related tests:
- 16 GenreTree tests (loading, path-based IDs, matching, pruning, deduplicated counts)
- 10 title-case tests
- 35+ CustomGenreParser tests (happy path, validation, edge cases)

---

## 10. Phase 8: Rust Core — mpv Playback

### File: `ramus-core/src/playback/mpv.rs`

#### Architecture
- Thin wrapper around libmpv C API
- **Must NOT run on main/UI thread** — mpv callbacks fire on mpv's internal thread
- Use raw `libmpv-sys` FFI if `libmpv2` crate doesn't cover all needed operations

#### Initialisation Options
```
vo=null                     # No video
vid=no                      # Skip video tracks
ao=coreaudio|wasapi|pipewire  # Platform-specific audio output
gapless-audio=yes           # True gapless
prefetch-playlist=yes       # Pre-buffer next
audio-buffer=0.5            # 500ms buffer
keep-open=no                # Advance on EOF (NOT "always")
idle=yes                    # Stay alive when idle
input-default-bindings=no
input-vo-keyboard=no
terminal=no
load-scripts=no
msg-level=all=warn
```

#### Observed Properties
| Property | Format | ID | Callback |
|----------|--------|-----|----------|
| `time-pos` | Double | 1 | `on_position_change(f64)` |
| `duration` | Double | 2 | `on_duration_change(f64)` |
| `pause` | Flag | 3 | `on_pause_change(bool)` |
| `playlist-pos` | Int64 | 5 | `on_playlist_pos_change(i64)` |
| `paused-for-cache` | Flag | 7 | `on_buffering_change(bool)` |
| `idle-active` | Flag | 9 | `on_idle_active()` |
| `cache-buffering-state` | Int64 | 10 | `on_cache_state_change(i64)` |

#### Events Handled
| Event | Action |
|-------|--------|
| `FileLoaded` | `on_file_loaded()` |
| `EndFile` | `on_file_ended(reason)` — reason: EOF/Stop/Quit/Error/Redirect |
| `PropertyChange` | Dispatch to appropriate callback based on reply_userdata |

#### Commands
| Method | mpv Command |
|--------|------------|
| `load_file(url, mode)` | `loadfile "{url}" "{replace|append|append-play}"` |
| `load_file_at(url, index)` | `loadfile "{url}" "insert-at" "{index}"` |
| `playlist_play_index(i)` | set property `playlist-pos` = i |
| `playlist_remove(i)` | `playlist-remove {i}` |
| `playlist_move(from, to)` | `playlist-move {from} {to}` |
| `seek(position)` | `seek {position} absolute` |
| `set_pause(flag)` | set property `pause` = flag |
| `set_volume(vol)` | set property `volume` = vol |
| `set_audio_filters(value)` | set property `af` = value |
| `stop()` | `stop` |
| `get_volume() -> f64` | get property `volume` |

#### Wakeup & Event Loop
- `mpv_set_wakeup_callback` → signals a channel/condvar
- Dedicated thread drains events via `mpv_wait_event` in a loop
- Callbacks dispatched to registered closures
- Shutdown flag (atomic bool) prevents use-after-free on drop

#### Thread Safety
- mpv client API is thread-safe (can call from any thread)
- Store handle as raw pointer, wrap in `Send + Sync` newtype
- Drop: set shutdown flag → destroy mpv handle

See [Appendix C](#appendix-c-mpv-property--command-reference) for full reference.

### File: `ramus-core/src/playback/transcode.rs`

#### `should_transcode(track, config, is_remote) -> bool`
- `DirectPlay` mode → always false
- `TranscodeLossless` mode → true if codec is lossless
- `TranscodeLosslessRemote` mode → true if codec is lossless AND connection is remote
- Lossless codecs: `{"flac", "alac", "wav", "aiff", "aif", "pcm"}`

#### `build_direct_play_url(server_url, part_key, token) -> Option<Url>`
- Validate: `part_key` starts with `/library/`, no `..` traversal
- Output: `{server_url}{part_key}?X-Plex-Token={token}`

#### `build_hls_url(server_url, token, rating_key, client_id, session) -> Option<Url>`
- Endpoint: `/music/:/transcode/universal/start.m3u8` (NOT `/audio/:/...`)
- Key params: `path=/library/metadata/{rk}`, `maxAudioBitrate=256`, `protocol=hls`
- Profile: `add-transcode-target(type=musicProfile&context=streaming&protocol=hls&container=mpegts&audioCodec=aac,mp3)`
- Additional: `X-Plex-Platform=Chrome` header (required for transcode endpoint)

### Verification
- Test mpv initialisation and teardown (no crashes)
- Test property observation (mock event delivery)
- Test `should_transcode` for all mode×codec×remote combinations
- Test URL builders (security validation, correct query params)

---

## 11. Phase 9: Rust Core — Audio Player & Queue

### File: `ramus-core/src/playback/player.rs`

#### State (observable, emitted via events)
```rust
pub struct AudioPlayerState {
    pub state: PlayerState,       // status, current_track, queue, queue_index
    pub position: f64,            // seconds, throttled ~30fps
    pub duration: f64,
    pub is_loading: bool,
    pub is_buffering: bool,
    pub waveform_levels: Option<Vec<f32>>,  // normalised 0..1
    pub buffered_fraction: f64,   // 0.0..1.0
    pub volume: f64,              // 0..100
}
```

#### Download Cache (LRU)
- Directory: `{temp_dir}/ramus_audio_cache/`
- Map: `track_id → local file path`
- Size tracking: `track_id → file_size_bytes`
- Access order: `Vec<track_id>` (oldest first)
- Eviction: remove oldest until total < `config.audio_cache_limit_bytes`
- Allowed extensions: `{flac, alac, m4a, mp3, aac, wav, aiff, ogg, opus, mp2, bin}`
- Filename sanitisation: `[a-zA-Z0-9_-]+` only

#### Queue Operations
| Method | Behaviour |
|--------|-----------|
| `load_queue(tracks, start_at)` | Replace queue, start playback, new session ID |
| `append_to_queue(tracks)` | Append, auto-start if stopped |
| `insert_next(tracks)` | Insert after current index |
| `remove_from_queue(index)` | Remove (not current track) |
| `jump_to_index(index)` | Seek to queue position |
| `next()` | Advance (stop at end) |
| `previous()` | Go back (stop at beginning) |
| `toggle_play_pause()` | Toggle |
| `seek(position)` | Absolute seek in seconds |
| `set_volume(vol)` | 0–100 |

#### Playlist Building (`build_and_load_playlist`)
1. Download starting track if direct-play (for instant start)
2. Build mpv playlist: first track with `"replace"`, rest with `"append"`
3. Start playback from `start_index` via `playlist_play_index`
4. Prefetch in background after playback starts

**Do NOT call `mpv.stop()` in `load_queue`** — `loadfile "replace"` already stops. Explicit stop races with playlist setup.

#### Prefetch
- Background task: download next N tracks (`config.lookahead_depth`)
- Skip HLS transcode tracks (mpv streams directly)
- Retry: 3 attempts with 1s/2s/3s backoff
- Rebuild URL on each retry (server connection may have changed)

#### Equalizer
- 10 bands: 31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000 Hz
- Gain: -12 to +12 dB
- Filter string: `lavfi=[equalizer=f=31:width_type=o:w=1:g={gain},...repeated for each band...]`
- **Use POSIX locale for float formatting** — comma decimal separators break lavfi
- Sanitise NaN/Inf → 0
- Clear: `mpv.set_audio_filters("no")` (not empty string)
- EQ persists across tracks (mpv `af` property survives `loadfile`)

#### Buffering
- Delay showing indicator by 300ms (avoids flash for cached/LAN tracks)
- `is_buffering = is_mpv_buffering || is_loading`

#### Callbacks (to Tauri event system)
- `on_track_change` — after playback starts
- `on_track_info_change` — metadata updated
- `on_playback_state_change(PlaybackStatus)` — pause/resume/stop
- `on_track_ended(Track)` — natural advance (for scrobbling)

### Verification
- Test queue operations (load, append, insert, remove, jump)
- Test LRU cache eviction
- Test equalizer filter string building (locale safety)
- Test download retry logic
- Test gapless advance (mpv playlist-pos change → correct queue index update)

---

## 12. Phase 10: Rust Core — Lyrics, Waveform, Session Reporting

### File: `ramus-core/src/playback/lyrics.rs`

#### Fetch Priority
1. Plex local `.lrc` sidecar → `fetch_from_plex(rating_key)`
2. LRCLIB fallback → `fetch_from_lrclib(track_name, artist_name, album_name, duration)`

#### Plex Lyrics
- Get stream info: `client.fetch_lyrics_stream(rating_key)` → find `stream_type == 4`
- **Skip LyricFind provider** (`provider == "com.plexapp.agents.lyricfind"` — DRM-gated)
- Validate path: no `..`, starts with `/library/` or `/file/`
- Download raw data: `client.download_lyrics_data(path)`
- Parse as:
  1. **Plex JSON**: `MediaContainer > Lyrics > Line > Span` with `start_offset` (ms)
  2. **LRC format** (if `format == "lrc"` or `timed == true`): `[MM:SS.cc] text`
  3. **Plain text**: unsynced

#### LRCLIB API
- `GET https://lrclib.net/api/get?track_name={}&artist_name={}&album_name={}&duration={secs}`
- Header: `Lrclib-Client: ramus v1.0.0 (https://github.com/1337raspberry/ramus)`
- Timeout: 10s, max response: 512KB
- Response: `{ plainLyrics: String?, syncedLyrics: String? }`
- Prefer `synced_lyrics` (LRC format), fallback to `plain_lyrics`

#### LRC Parsing
- Pattern: `[MM:SS.cc] text` where cc is optional centiseconds
- Timestamp: `minutes * 60 + seconds`
- Skip empty lines after trimming

#### Result Type
```rust
pub struct LyricsResult {
    pub lines: Vec<LyricLine>,  // { id, timestamp: Option<f64>, text }
    pub is_synced: bool,
    pub source: LyricsSource,   // Plex or Lrclib
}
impl LyricsResult {
    pub fn active_line_index(&self, position: f64) -> Option<usize>
    // Binary search: highest line.timestamp <= position
}
```

### File: `ramus-core/src/playback/waveform.rs`

```rust
pub fn normalize(db_levels: &[f32]) -> Vec<f32> {
    // 1. Convert dB to linear: pow(10, db / 20.0)
    // 2. Divide by max → 0..1
}
```

### File: `ramus-core/src/playback/session.rs`

#### Lifecycle
- Track one active session: `active_session_id` + `active_track`
- **Always send `state=stopped` for previous track before reporting new one**

#### Methods
| Method | Action |
|--------|--------|
| `track_started(track, session_id)` | Stop previous → report `state=playing` |
| `track_ended(track)` | Scrobble via `client.scrobble()` |
| `playback_paused()` | Report `state=paused`, stop periodic timer |
| `playback_resumed()` | Report `state=playing`, restart timer |
| `playback_stopped()` | Report `state=stopped`, clear state |
| `playback_seeked(position)` | Immediate position update |

#### Periodic Reporting
- Every 10 seconds while playing
- Check scrobble: if `progress >= 0.9` → `client.scrobble()` (once per track)

#### App Termination
- Synchronous `state=stopped` report with 2s timeout
- Build request inline (can't use async client during shutdown)

### Verification
- Test LRC parsing (various formats, edge cases)
- Test LRCLIB response parsing
- Test `active_line_index` binary search
- Test waveform normalisation
- Test session reporter lifecycle (no ghost sessions)
- Test scrobble fires at 90% and only once

---

## 13. Phase 11: Rust Core — Connection Monitor & Media Keys

### File: `ramus-core/src/plex/connection.rs`

#### Connection Monitoring
- Monitor network changes (platform-specific)
  - macOS: `SCNetworkReachability` or poll-based
  - Linux: `netlink` socket or poll-based
  - Windows: `NotifyAddrChange` or poll-based
- Debounce: 500ms (coalesce rapid changes like VPN toggles)
- Only re-evaluate if interface set changes

#### Failover Priority
1. Test current connection (3s timeout)
2. Try cached connections sorted by priority (5s timeout each)
3. Re-discover from plex.tv as last resort
4. Signal connection lost if nothing works

#### Concurrent Testing
- Test all connections concurrently (avoid sequential 5s timeouts)
- Return highest-priority working connection

### File: `ramus-core/src/playback/media_keys.rs`

#### `souvlaki` Crate
- Cross-platform media key integration:
  - macOS: `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`
  - Windows: `SystemMediaTransportControls`
  - Linux: MPRIS D-Bus
- Register handlers: play, pause, toggle, next, previous, seek
- Update metadata: title, artist, album, duration, position, playback rate
- Artwork: load from URL, set as platform media artwork

### Verification
- Test failover priority ordering
- Test concurrent connection testing
- Test debounce (rapid changes coalesced)
- Test media key metadata updates

---

## 14. Phase 12: Tauri Shell & IPC

**Goal:** Wire the Rust core to the React frontend via Tauri commands and events.

### File: `ramus-tauri/src/state.rs`

```rust
pub struct AppState {
    pub client: Arc<PlexClient>,
    pub cache: Arc<Mutex<Option<CacheDatabase>>>,
    pub player: Arc<AudioPlayer>,
    pub genre_mapper: Arc<RwLock<Option<GenreMapper>>>,
    pub search_engine: Arc<RwLock<Option<SearchEngine>>>,
    pub sync_engine: Arc<SyncEngine>,
    pub session_reporter: Arc<SessionReporter>,
    pub connection_monitor: Arc<ConnectionMonitor>,
    pub settings: Arc<RwLock<Settings>>,
}
```

### File: `ramus-tauri/src/events.rs`

Events emitted from Rust → frontend (via `app.emit()`):

| Event Name | Payload | Frequency |
|------------|---------|-----------|
| `playback-state` | `{ status, current_track, queue_index }` | On state change |
| `playback-position` | `{ position, duration }` | ~30fps during playback |
| `playback-buffering` | `{ is_buffering, buffered_fraction }` | On change |
| `waveform-data` | `{ levels: Vec<f32> }` | On track change |
| `sync-progress` | `SyncProgress` | During sync |
| `connection-changed` | `{ server_url, is_local, is_http }` | On failover |
| `connection-lost` | `{}` | When all connections fail |
| `accent-color` | `{ r, g, b }` | On track change (extracted from art) |
| `lyrics-update` | `LyricsResult` | On fetch complete |

### Commands (by file)

#### `commands/auth.rs`
```rust
#[tauri::command] async fn start_oauth(state) -> Result<String>      // Returns auth URL
#[tauri::command] async fn poll_oauth(state, pin_id) -> Result<bool>  // Returns true when done
#[tauri::command] async fn discover_servers(state) -> Result<Vec<PlexServer>>
#[tauri::command] async fn test_server(state, server) -> Result<ConnectionInfo>
#[tauri::command] async fn connect_manual_url(state, url) -> Result<bool>
#[tauri::command] async fn find_music_libraries(state) -> Result<Vec<LibrarySection>>
#[tauri::command] async fn finalize_onboarding(state, config) -> Result<()>
#[tauri::command] async fn is_authenticated(state) -> Result<bool>
#[tauri::command] async fn logout(state) -> Result<()>
```

#### `commands/library.rs`
```rust
#[tauri::command] async fn get_genre_tree(state) -> Result<Vec<GenreNode>>
#[tauri::command] async fn get_albums_for_genre(state, genre) -> Result<Vec<Album>>
#[tauri::command] async fn get_all_albums(state) -> Result<Vec<Album>>
#[tauri::command] async fn get_favourite_albums(state) -> Result<Vec<Album>>
#[tauri::command] async fn get_albums_for_artist(state, source_id) -> Result<Vec<Album>>
#[tauri::command] async fn get_tracks_for_album(state, source_id) -> Result<Vec<Track>>
#[tauri::command] async fn get_all_artists(state) -> Result<Vec<ArtistInfo>>
#[tauri::command] async fn get_favourite_genre_tree(state) -> Result<Vec<GenreNode>>
#[tauri::command] async fn toggle_album_favourite(state, source_id, favourite) -> Result<()>
#[tauri::command] async fn toggle_track_favourite(state, source_id, favourite) -> Result<()>
#[tauri::command] async fn get_album_genres(state, source_id) -> Result<Vec<String>>
#[tauri::command] async fn get_random_album(state) -> Result<Option<Album>>
#[tauri::command] async fn get_art_url(state, thumb, size) -> Result<String>
#[tauri::command] async fn get_cache_stats(state) -> Result<CacheStats>
```

#### `commands/playback.rs`
```rust
#[tauri::command] async fn play_tracks(state, tracks, start_at) -> Result<()>
#[tauri::command] async fn toggle_play_pause(state) -> Result<()>
#[tauri::command] async fn next_track(state) -> Result<()>
#[tauri::command] async fn previous_track(state) -> Result<()>
#[tauri::command] async fn seek(state, position) -> Result<()>
#[tauri::command] async fn set_volume(state, volume) -> Result<()>
#[tauri::command] async fn get_volume(state) -> Result<f64>
#[tauri::command] async fn append_to_queue(state, tracks) -> Result<()>
#[tauri::command] async fn insert_next(state, tracks) -> Result<()>
#[tauri::command] async fn remove_from_queue(state, index) -> Result<()>
#[tauri::command] async fn jump_to_queue_index(state, index) -> Result<()>
#[tauri::command] async fn get_queue(state) -> Result<Vec<Track>>
#[tauri::command] async fn apply_equalizer(state, enabled, bands) -> Result<()>
#[tauri::command] async fn fetch_lyrics(state, rating_key) -> Result<Option<LyricsResult>>
#[tauri::command] async fn get_waveform(state, rating_key) -> Result<Option<Vec<f32>>>
```

#### `commands/search.rs`
```rust
#[tauri::command] async fn search(state, query, limit) -> Result<Vec<SearchResult>>
```

#### `commands/sync.rs`
```rust
#[tauri::command] async fn start_full_sync(state) -> Result<()>
#[tauri::command] async fn start_incremental_sync(state) -> Result<()>
#[tauri::command] async fn start_genre_sync(state) -> Result<()>
```

#### `commands/settings.rs`
```rust
#[tauri::command] async fn get_settings(state) -> Result<Settings>
#[tauri::command] async fn update_settings(state, settings) -> Result<()>
#[tauri::command] async fn import_custom_genres(state, text, filename) -> Result<Vec<String>> // warnings
#[tauri::command] async fn remove_custom_genres(state) -> Result<()>
```

### Window Configuration (`tauri.conf.json`)
```json
{
  "app": {
    "windows": [{
      "title": "ramus",
      "width": 1200,
      "height": 800,
      "minWidth": 800,
      "minHeight": 500,
      "decorations": false,
      "transparent": true
    }]
  }
}
```
- Titlebar hidden (custom drag area in React)
- Dark theme enforced
- Scrollbars hidden via CSS

### Verification
- Test each command can be invoked and returns correct types
- Test event emission (mock subscriber)
- Test state initialisation and auto-connect flow

---

## 15. Phase 13: React Frontend — Layout & Navigation

### ThreeColumnLayout.tsx
- CSS Grid with three columns
- Draggable dividers (onMouseDown → onMouseMove → onMouseUp)
- Column widths: sidebar 180–350px (default 220), content min 200px, detail 280–800px (default 420)
- Cursor: `col-resize` on divider hover
- Persist widths to localStorage

### SidebarView.tsx
- Three modes: Genres / Favourites / Artists (tab buttons at top)
- Mode-specific content area
- Genres mode: `GenreTreeView` component
- Favourites mode: `GenreTreeView` with favourite-filtered tree
- Artists mode: virtualised artist list

### App.tsx
- Check `is_authenticated()` → show Onboarding or main layout
- Auto-connect on mount if authenticated
- Global keyboard shortcuts:
  - `Cmd/Ctrl+F` → open search overlay
  - Space → toggle play/pause
  - `Cmd/Ctrl+Right` → next track
  - `Cmd/Ctrl+Left` → previous track
- Window drag area (top bar, data-tauri-drag-region)

### CSS Foundation
- Dark theme (force dark mode)
- CSS variables for accent color (updated from `accent-color` event)
- `--accent-r`, `--accent-g`, `--accent-b` → used in `rgb()` and `rgba()`
- Hide scrollbars globally (`::-webkit-scrollbar { display: none }`)
- Glass morphism via `backdrop-filter: blur(...)` + semi-transparent backgrounds

---

## 16. Phase 14: React Frontend — Genre Tree & Album Grid

### GenreTreeView.tsx
- **Virtualised** with `@tanstack/react-virtual` (6,250+ nodes)
- Recursive row rendering with indentation: `paddingLeft = depth * 8px`
- Manual chevron icons (not native disclosure)
- Expand/collapse: toggle node ID in `Set<string>`
- "All" sentinel node at top (expand/collapse all)
- Album count badge on right
- Click → select genre → load albums
- Multi-location cycling: `navigateToGenre` cycles through matches, `scrollIntoView`

### AlbumGridView.tsx
- CSS Grid: `grid-template-columns: repeat(auto-fill, minmax(125px, 1fr))`, gap 16px
- Sort menu (top-right): Alphabetical / Latest Added / Recently Played / Random
- Album cards:
  - Album art (async loaded, placeholder on error)
  - Title + artist text below
  - Double-click → play album
  - Hover → browse button + favourite star
  - Selection highlight (accent at 8% opacity)
- Favourite star: always visible when favourited, hover-only otherwise

### TrackListView.tsx
- Track list for selected album in content column
- Track number, title, artist (if different), duration
- Click → play album starting at track
- Favourite toggle per track
- Format badge (FLAC, MP3, etc.)

### Zustand Stores

**libraryStore.ts:**
```typescript
interface LibraryState {
  sidebarMode: 'genres' | 'favourites' | 'artists'
  genreTree: GenreNode[]
  expandedGenreIds: Set<string>
  selectedGenre: GenreNode | null
  albums: Album[]
  selectedAlbum: Album | null
  tracks: Track[]
  albumSortOrder: 'alphabetical' | 'latestAdded' | 'recentlyPlayed' | 'random'
  // Actions...
}
```

---

## 17. Phase 15: React Frontend — Now Playing, Waveform, Lyrics

### NowPlayingView.tsx
Layout (top to bottom):
1. **Header:** Artist name (clickable), album title, year, favourite star
2. **Album art** (large, tap to flip to lyrics)
3. **Volume slider** (thin, accent-coloured)
4. **Track title**, EQ button, track favourite
5. **WaveformSeekBar** component
6. **Transport:** previous / play-pause / next buttons
7. **Footer:** Genre pills (FlowLayout), studio, format info

### LyricsView.tsx
- Overlay on album art (spring animation: scale 0→1)
- Scrollable lyrics list
- Active line highlighted (binary search by position)
- Auto-scroll to active line
- Pin toggle (auto-fetch on track change)

### WaveformSeekBar.tsx
- **Canvas-based** drawing
- Quad-curve smoothed waveform path
- Three regions: played (accent 85%), buffered (secondary 35%), unplayed (secondary 20%)
- Drag to seek (horizontal, clamped 0–1)
- Time labels (monospaced): elapsed / total
- Buffering animation: scanning gradient sweep

### QueueView.tsx
- "Up Next" header with count
- Virtualised list of upcoming tracks
- Each row: thumbnail, title, artist, duration, remove button
- Click → jump to queue index
- Pagination: show 30, "Show more" loads 50 more

### VolumeSlider.tsx
- Thin line track with circular thumb
- Accent colour fill
- Drag to adjust (0–100)

### FlowLayout.tsx
- Genre pills wrap to next row (CSS flexbox with `flex-wrap: wrap`)
- Each pill is a button → navigate to genre in sidebar
- Hover: cursor pointer

---

## 18. Phase 16: React Frontend — Search, EQ, Queue, Settings

### SearchOverlay.tsx
- Floating overlay (portal, centered)
- Width: 350px (empty) → 500px (with results)
- Glass morphism background
- Search input with magnifying glass icon, monospace font
- 150ms debounce on input
- Results: albums section (max 5) then tracks section
- Each result: thumbnail, title, artist, action buttons (play next, add to queue)
- Keyboard: ↑/↓ navigate, Return select, Shift+Return load all albums, Escape dismiss
- Selected row: accent background 20% opacity

### EqualizerPanel.tsx
- Floating panel (CSS position: fixed, portal)
- 380 x 280px
- Header: "Equalizer" title, toggle switch, close button
- 10 vertical sliders (31Hz – 16kHz)
- dB scale: +12 to -12
- Snap to 0 when near (< 0.8 dB)
- 0.5 dB quantization
- Filled track from centre to thumb (accent colour)
- Reset button
- Click outside / Escape → dismiss
- 50ms debounce on slider drag → `apply_equalizer` command

### LibrarySettingsPanel.tsx
- Server info, sync controls, preferences
- Playback mode picker (Direct Play / Transcode Lossless Remote / Transcode Lossless)
- Lookahead depth slider (1–20)
- Audio cache limit (0.1–50 GB)
- Sync interval (0=disabled, or hours)
- Genre source: Open (default) / Custom (file import)
- Library padding slider (4–10)
- Show taglines toggle
- Refuse HTTP toggle
- Cache stats display
- Manual sync buttons (Full / Incremental / Genre-only)

---

## 19. Phase 17: React Frontend — Onboarding

### OnboardingFlow.tsx
- Step state machine: `signIn → discoverServers → selectLibrary → initialSync`
- Card layout (centred, semi-transparent, rounded)
- Transition animation between steps

### OAuthSignIn.tsx
- "Sign in with Plex" button → `start_oauth` command → opens browser
- Poll with `poll_oauth` every 2s
- Show PIN code during poll
- Error display

### ServerPicker.tsx
- Server list with icons (owned/shared), name, connection info
- Selected server highlighted
- Connection testing progress
- Connection type display (Local/Remote/Relay/Manual)
- Manual URL entry (expandable section)
- HTTP warning alert

### LibraryPicker.tsx
- Library list with checkmark on selection
- Server name subtitle

### InitialSync.tsx
- Progress bar (deterministic fraction)
- Phase label + detail text
- "Start Sync" / "Skip for now" / "Get Started" buttons

---

## 20. Phase 18: Polish, Platform Packaging, Testing

### End-to-End Functional Test
1. OAuth → server select → library sync → genre browsing → album grid → track playback → gapless advance → search with operators → lyrics display → waveform seeking → equalizer

### Performance Targets
- Genre tree (6,250+ nodes): renders without jank
- Search results: < 100ms
- Gapless playback: no audible gaps
- Binary size: < 20MB per platform

### Platform Packaging

**macOS:**
- `cargo tauri build` → `.dmg` or `.app`
- Bundle `libmpv.dylib` (Homebrew or build from source)
- Code signing + notarisation
- Distribution: Homebrew cask

**Windows:**
- `cargo tauri build` → NSIS installer or MSI
- Bundle `mpv-2.dll`
- Distribution: WinGet

**Linux:**
- `cargo tauri build` → AppImage or `.deb`
- Require system `libmpv` (or bundle `.so`)
- Distribution: Flatpak, AppImage

### Tauri Auto-Updater
- Configure `tauri-plugin-updater`
- Signed updates per platform

---

## Appendix A: Full Database Schema

```sql
CREATE TABLE artists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    sortName TEXT,
    sourceId TEXT NOT NULL UNIQUE,
    artUrl TEXT,
    summary TEXT,
    updatedAt INTEGER
);
CREATE INDEX idx_artists_name ON artists(name);

CREATE TABLE albums (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    artistId INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    year INTEGER,
    sourceId TEXT NOT NULL UNIQUE,
    artUrl TEXT,
    updatedAt INTEGER,
    rating DOUBLE,
    studio TEXT,
    ultraBlurColors TEXT,
    addedAt INTEGER,
    lastViewedAt INTEGER
);
CREATE INDEX idx_albums_title ON albums(title);

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    albumId INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    artistId INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    trackNumber INTEGER,
    discNumber INTEGER,
    durationMs INTEGER,
    sourceId TEXT NOT NULL UNIQUE,
    codec TEXT,
    partKey TEXT,
    updatedAt INTEGER,
    streamId INTEGER,
    userRating DOUBLE,
    bitrate INTEGER,
    trackArtist TEXT
);
CREATE INDEX idx_tracks_albumId ON tracks(albumId);

CREATE VIRTUAL TABLE tracks_fts USING FTS5(
    content='tracks',
    title,
    tokenizer='unicode61',
    prefixes=[2,3]
);

CREATE TABLE genres (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE album_genres (
    albumId INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    genreId INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (albumId, genreId)
);
CREATE INDEX idx_album_genres_genreId ON album_genres(genreId);
```

### Upsert SQL Patterns

**Artist:**
```sql
INSERT INTO artists (name, sortName, sourceId, artUrl, summary, updatedAt)
VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(sourceId) DO UPDATE SET
    name = excluded.name,
    sortName = excluded.sortName,
    artUrl = excluded.artUrl,
    summary = excluded.summary,
    updatedAt = excluded.updatedAt;
```

**Album:**
```sql
INSERT INTO albums (title, artistId, year, sourceId, artUrl, updatedAt, addedAt, lastViewedAt)
VALUES (?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(sourceId) DO UPDATE SET
    title = excluded.title,
    artistId = excluded.artistId,
    year = excluded.year,
    artUrl = excluded.artUrl,
    rating = COALESCE(excluded.rating, albums.rating),
    studio = COALESCE(excluded.studio, albums.studio),
    updatedAt = excluded.updatedAt,
    addedAt = COALESCE(excluded.addedAt, albums.addedAt),
    lastViewedAt = COALESCE(excluded.lastViewedAt, albums.lastViewedAt);
```

**Track:**
```sql
INSERT INTO tracks (title, albumId, artistId, trackNumber, discNumber, durationMs, sourceId, codec, partKey, streamId, userRating, bitrate, trackArtist, updatedAt)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(sourceId) DO UPDATE SET
    title = excluded.title,
    albumId = excluded.albumId,
    artistId = excluded.artistId,
    trackNumber = excluded.trackNumber,
    discNumber = excluded.discNumber,
    durationMs = excluded.durationMs,
    codec = excluded.codec,
    partKey = excluded.partKey,
    streamId = excluded.streamId,
    userRating = excluded.userRating,
    bitrate = excluded.bitrate,
    trackArtist = excluded.trackArtist,
    updatedAt = excluded.updatedAt;
```

---

## Appendix B: Plex API Endpoint Reference

### Headers (all requests)
```
Accept: application/json
X-Plex-Client-Identifier: {uuid}
X-Plex-Product: ramus
X-Plex-Platform: {macOS|Windows|Linux}
X-Plex-Device: {Mac|PC|Linux}
X-Plex-Token: {token}
```

### Endpoints

| Method | URL | Notes |
|--------|-----|-------|
| GET | `https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=1` | Server discovery (auth token in header) |
| POST | `https://plex.tv/api/v2/pins?strong=true&X-Plex-Product=ramus&X-Plex-Client-Identifier={id}` | Create OAuth PIN |
| GET | `https://plex.tv/api/v2/pins/{pin_id}` | Poll for OAuth token |
| GET | `{server}/identity` | Connection test |
| GET | `{server}/library/sections` | List libraries |
| GET | `{server}/library/sections/{key}/all?type={8,9,10}&X-Plex-Container-Start={n}&X-Plex-Container-Size={n}` | Paginated items |
| GET | `{server}/library/metadata/{ratingKey}` | Full item metadata |
| GET | `{server}/library/streams/{streamId}/levels?subsample={n}` | Waveform loudness |
| PUT | `{server}/:/timeline?ratingKey={rk}&key=/library/metadata/{rk}&state={playing,paused,stopped}&time={ms}&duration={ms}&identifier=com.plexapp.plugins.library&X-Plex-Token={t}` | Timeline report (+ X-Plex-Session-Identifier header) |
| PUT | `{server}/:/scrobble?key=/library/metadata/{rk}&identifier=com.plexapp.plugins.library&X-Plex-Token={t}` | Mark as played |
| PUT | `{server}/:/rate?key={rk}&identifier=com.plexapp.plugins.library&rating={0,10}&X-Plex-Token={t}` | Favourite toggle |
| GET | `{server}/{partKey}?X-Plex-Token={t}` | Direct play audio |
| GET | `{server}/music/:/transcode/universal/start.m3u8?path=/library/metadata/{rk}&maxAudioBitrate=256&protocol=hls&...` | HLS transcode (NOT `/audio/:/transcode/...`) |

### Plex JSON Structure

**MediaItem keys** (JSON key → struct field):
- `ratingKey`, `title`, `titleSort`, `originalTitle`, `summary`
- `parentTitle`, `grandparentTitle`, `parentRatingKey`, `grandparentRatingKey`
- `index` (track#), `parentIndex` (disc#)
- `year`, `duration` (**milliseconds!**), `updatedAt`, `addedAt`, `lastViewedAt`
- `thumb`, `parentThumb`, `grandparentThumb`, `art`
- `userRating` (0–10), `studio`
- `Media` → array of `{ audioCodec, bitrate, Part: [{ key, Stream: [{ id, streamType, codec, bitrate, key, format, timed, provider }] }] }`
- `Genre` → array of `{ tag }`
- `ultraBlurColors` → `{ topLeft, topRight, bottomRight, bottomLeft }`

**StreamInfo.timed** can be JSON bool OR int — handle both in deserializer.

**Type codes:** 8=artist, 9=album, 10=track. Stream type 2=audio, 4=lyrics.

---

## Appendix C: mpv Property & Command Reference

### Properties Set at Init
| Property | Value | Purpose |
|----------|-------|---------|
| `vo` | `null` | No video output |
| `vid` | `no` | Skip video tracks |
| `ao` | `coreaudio` / `wasapi` / `pipewire` | Platform audio output |
| `gapless-audio` | `yes` | True gapless between tracks |
| `prefetch-playlist` | `yes` | Buffer next entry |
| `audio-buffer` | `0.5` | 500ms audio buffer |
| `keep-open` | `no` | Advance on EOF (NOT `always` — pauses instead) |
| `idle` | `yes` | Stay alive when idle |
| `input-default-bindings` | `no` | Disable key bindings |
| `input-vo-keyboard` | `no` | Disable keyboard |
| `terminal` | `no` | Disable terminal output |
| `load-scripts` | `no` | No Lua scripts |
| `msg-level` | `all=warn` | Reduce log noise |

### Properties Observed
| Property | Format | Use |
|----------|--------|-----|
| `time-pos` | double | Current playback position (seconds) |
| `duration` | double | Track duration (seconds) |
| `pause` | flag | Pause state |
| `playlist-pos` | int64 | Current playlist entry index |
| `paused-for-cache` | flag | True when buffering |
| `idle-active` | flag | True when idle (no file loaded) |
| `cache-buffering-state` | int64 | 0–100 buffering percentage |

### Properties Read/Written at Runtime
| Property | Direction | Use |
|----------|-----------|-----|
| `volume` | R/W | Volume 0–100+ |
| `af` | W | Audio filter chain (equalizer) |
| `playlist-pos` | W | Jump to playlist entry |
| `pause` | W | Set pause state |

### Commands Used
| Command | Args | Use |
|---------|------|-----|
| `loadfile` | `url, mode` | Load file. Mode: `replace`, `append`, `append-play`, `insert-at {index}` |
| `playlist-remove` | `index` | Remove playlist entry |
| `playlist-move` | `from, to` | Reorder playlist |
| `seek` | `position, absolute` | Absolute seek |
| `stop` | — | Clear playlist and stop |

### Equalizer Filter String
```
lavfi=[equalizer=f=31:width_type=o:w=1:g=0.0,equalizer=f=62:width_type=o:w=1:g=0.0,...,equalizer=f=16000:width_type=o:w=1:g=0.0]
```
Clear filters: `af=no` (not empty string)

### File End Reasons
| Reason | Meaning |
|--------|---------|
| `MPV_END_FILE_REASON_EOF` | Natural end |
| `MPV_END_FILE_REASON_STOP` | User stop |
| `MPV_END_FILE_REASON_QUIT` | Player shutdown |
| `MPV_END_FILE_REASON_ERROR` | Playback error |
| `MPV_END_FILE_REASON_REDIRECT` | URL redirect |

---

## Appendix D: Test Specification

All tests ported from the Swift test suite (~122 tests).

### QueryParser Tests (23 tests)
```
test_parse_free_text
test_parse_genre_filter
test_parse_artist_filter
test_parse_album_title_filter
test_parse_track_search
test_parse_multi_word_track_search
test_parse_multi_word_album_title_search
test_parse_year_greater_than
test_parse_year_equal
test_parse_rating_greater_or_equal
test_parse_less_than_or_equal
test_parse_combined_with_and
test_multi_word_genre_without_and
test_multi_word_artist_without_and
test_operator_without_and_consumes_all
test_parse_empty_input
test_parse_invalid_year
test_parse_bare_operators
test_escape_fts5
test_escape_fts5_hyphen_replaced_with_space
test_escape_fts5_keywords_neutralised_by_quoting
test_default_search_does_not_produce_track_search
test_is_free_text_only
```

### SearchEngine Tests (13 tests)
```
test_free_text_search_returns_albums_and_tracks
test_artist_filter_returns_albums
test_album_title_filter
test_genre_filter_returns_albums
test_genre_filter_expands_hierarchy
test_year_range_filter
test_combined_filters
test_track_search_returns_tracks_only
test_track_search_fuzzy_fallback        # "paranoyd" → "Paranoid Android"
test_free_text_albums_appear_before_tracks
test_free_text_gibberish_returns_empty
test_free_text_albums_capped_at_five
test_empty_query
```

### GenreTree Tests (16 tests)
```
test_load_tree_from_json
test_leaf_nodes_have_none_children
test_path_based_ids
test_duplicate_genre_names_have_unique_ids
test_exact_match_case_insensitive
test_exact_match_mixed_case
test_fuzzy_match_close_spelling          # "Progressve Rock"
test_fuzzy_match_slight_typo             # "Deth Metal"
test_no_match_returns_none
test_build_display_tree_prunes_empty_branches
test_build_display_tree_empty_sets
test_deduplicated_count_with_shared_album
test_deduplicated_count_parent_and_child
test_all_descendant_names
```

### Title Case Tests (10 tests)
```
test_all_lowercase_gets_title_cased      # "ambient music" → "Ambient Music"
test_acronym_left_alone                  # "EBM" → "EBM"
test_ampersand_acronym_left_alone        # "R&B" → "R&B"
test_hyphenated_compound                 # "lo-fi" → "Lo-Fi"
test_mixed_case_per_word_preservation    # "death Metal" → "Death Metal"
test_word_with_existing_uppercase        # "dEath metal" → "dEath Metal"
test_single_lowercase_word               # "jazz" → "Jazz"
test_empty_string_returns_empty
test_multiple_hyphen_segments            # "drum-and-bass" → "Drum-And-Bass"
test_mixed_hyphen_segments               # "lo-FI" → "Lo-FI"
```

### CustomGenreParser Tests (35+ tests)
```
# Happy path (6)
test_basic_hierarchy
test_tab_indentation
test_four_space_indentation
test_optional_descriptions
test_empty_brackets_no_description
test_leaf_nodes_have_none_children

# Validation & errors (8)
test_empty_file
test_whitespace_only_file
test_file_too_large
test_too_many_lines
test_indentation_jump
test_unmatched_bracket
test_name_too_long
test_json_input_rejected
test_json_array_input_rejected
test_no_root_genres

# Warnings (3)
test_duplicate_name_warning
test_duplicate_case_insensitive
test_duplicates_allowed_across_parents

# Edge cases (13)
test_blank_lines_ignored
test_c0_control_characters_stripped
test_c1_control_characters_stripped
test_unicode_preserved
test_mixed_tabs_and_spaces_uses_detected_unit
test_leaf_nodes_have_nil_children
test_single_root_genre
test_deeply_nested                       # 8 levels deep
test_description_with_brackets_inside
test_description_only_line_skipped
test_same_child_name_under_different_parents_no_warning
test_root_duplicate_still_detected
test_round_trip_through_genre_mapper
```

### Cache Tests (14 tests)
```
test_artist_crud
test_album_crud
test_track_crud
test_multiple_genres_per_album
test_genre_upsert_case_insensitivity
test_batch_operations
test_timestamp_queries
test_fts5_prefix_search                  # "par"* → "Paranoid Android"
test_like_pattern_escaping (9 sub-tests)
test_album_year_range_filters (11 sub-tests)
```

### Mock Data (for seeding tests)
- **Artists:** Radiohead, Slayer, Bjork
- **Albums:** OK Computer (Radiohead, 1997), Kid A (Radiohead, 2000), Reign in Blood (Slayer, 1986)
- **Tracks:** Paranoid Android, Everything in its Right Place, Raining Blood, etc.
- **Genres:** Rock → Alternative Rock → Shoegaze, Metal → Thrash Metal → Crossover Thrash, Electronic

---

## Appendix E: Platform-Specific Concerns

| Concern | macOS | Windows | Linux |
|---------|-------|---------|-------|
| **mpv library** | Homebrew `brew install mpv` or bundled dylib | Bundled `mpv-2.dll` | System `libmpv-dev` or bundled `.so` |
| **Audio output** | `ao=coreaudio` | `ao=wasapi` | `ao=pipewire` (fallback `pulse`) |
| **Hardware UUID** | `IOPlatformUUID` via IOKit | `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` | `/etc/machine-id` |
| **Media keys** | souvlaki (MPNowPlayingInfoCenter) | souvlaki (SystemMediaTransportControls) | souvlaki (MPRIS D-Bus) |
| **Config dir** | `~/Library/Application Support/ramus/` | `%APPDATA%\raspsoft\ramus\` | `~/.local/share/ramus/` |
| **Temp dir** | `$TMPDIR/ramus_audio_cache/` | `%TEMP%\ramus_audio_cache\` | `/tmp/ramus_audio_cache/` |
| **Tray icon** | Tauri built-in | Tauri built-in | Tauri built-in |
| **Notifications** | Tauri built-in | Tauri built-in | Tauri built-in |
| **Auto-update** | Tauri updater (.tar.gz + signature) | Tauri updater (NSIS/MSI) | AppImage / Flatpak |
| **Window chrome** | `decorations: false` + custom titlebar | Same | Same |
| **File permissions** | `0o600` on token file | Default ACL | `0o600` on token file |
| **Browser open** | `tauri-plugin-shell` | `tauri-plugin-shell` | `tauri-plugin-shell` |

### mpv Bundling Strategy

**macOS:** Bundle `libmpv.dylib` + dependencies in `Frameworks/`. Set `@rpath`. Entitlements: `com.apple.security.cs.disable-library-validation` (unsigned dylib).

**Windows:** Bundle `mpv-2.dll` + dependencies alongside executable. Use `mpv-dev` builds from https://sourceforge.net/projects/mpv-player-windows/

**Linux:** Prefer system `libmpv` (`apt install libmpv-dev`, `pacman -S mpv`). For AppImage, bundle `.so` files with `linuxdeploy`.

---

## Iteration Checkpoints

After each phase, verify before moving on:

| Phase | Checkpoint |
|-------|-----------|
| 1 (Models) | `cargo test -p ramus-core` — all model tests pass |
| 2 (API Client) | Unit tests with mock HTTP pass |
| 3 (Token/Auth) | Token round-trip + PIN parsing tests pass |
| 4 (Cache) | All CRUD, FTS5, range filter tests pass |
| 5 (Sync) | Incremental sync + progress tests pass |
| 6 (Search) | All 36 query parser + search engine tests pass |
| 7 (Genre) | All 61 genre tests pass |
| 8 (mpv) | mpv initialises, loads file, reports position (manual test) |
| 9 (Player) | Queue operations + EQ filter string tests pass |
| 10 (Lyrics etc.) | LRC parsing + session lifecycle tests pass |
| 11 (Connection) | Failover + media key tests pass |
| 12 (Tauri IPC) | `cargo tauri dev` boots, commands return data |
| 13 (Layout) | Three columns render, dividers drag |
| 14 (Genre+Grid) | Genre tree renders 6250+ nodes, albums load |
| 15 (Now Playing) | Track plays, waveform renders, lyrics display |
| 16 (Search+EQ) | Search operators work, EQ sliders apply |
| 17 (Onboarding) | Full OAuth → sync → play flow works |
| 18 (Polish) | All platforms build, binary < 20MB, gapless works |

---

## Working Process Summary

1. **Phases 1–7** are pure Rust with zero UI. Each phase produces a tested module. Run `cargo test` after each. These can often be developed in partial parallel (e.g., cache and genre tree are independent).

2. **Phases 8–11** introduce system dependencies (mpv, platform APIs). Test with integration tests against a real Plex server where possible, unit tests with mocks elsewhere.

3. **Phase 12** wires everything together via Tauri. This is the integration point — verify all commands work with `cargo tauri dev`.

4. **Phases 13–17** build the React frontend incrementally. Each phase adds a functional slice that can be tested visually.

5. **Phase 18** is packaging and cross-platform validation.

The key principle: **each phase produces something testable**. Never move to the next phase with failing tests. The Rust core should be fully functional and tested before any frontend work begins.
