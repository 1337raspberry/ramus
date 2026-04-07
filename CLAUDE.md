# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**ramus** is a cross-platform desktop music player for Plex media servers, being rewritten from a native Swift/SwiftUI macOS app into **Rust + Tauri 2 + React (TypeScript)**. The full specification lives in `IMPLEMENTATION_PLAN.md` — treat it as the authoritative source for all architecture, API contracts, data models, and test expectations.

The existing Swift codebase under `swift-project/` serves as the reference implementation. It contains two parts:
- `swift-project/ramus/` — the SwiftUI macOS app (views, view models, utilities)
- `swift-project/RamusMusicCore/` — SPM package with all business logic (PlexAPI, Playback, Cache, Search, GenreTree, Models)

## Target Architecture (Rust + Tauri + React)

```
ramus/
├── ramus-core/          # Rust library crate — all business logic
│   └── src/
│       ├── models.rs    # Shared types (Album, Track, PlayerState, etc.)
│       ├── plex/        # HTTP client, OAuth, token store, connection monitor
│       ├── playback/    # mpv wrapper, audio player, queue, EQ, lyrics, waveform, session reporting
│       ├── cache/       # rusqlite SQLite (WAL, FTS5), sync engine
│       ├── search/      # Query parser (operator syntax), search engine
│       └── genre/       # Genre tree, fuzzy mapper, custom genre parser
├── ramus-tauri/         # Tauri 2 app shell — commands, events, state management
├── ui/                  # React frontend — Vite + TypeScript + Zustand
└── scripts/             # build_open_genres.py (Wikidata SPARQL → genre JSON)
```

## Build & Test Commands

### Rust Core
```sh
cargo test -p ramus-core                    # Run all core tests
cargo test -p ramus-core -- test_name       # Run a single test
cargo test -p ramus-core -- module::        # Run tests in a module
cargo build -p ramus-core                   # Build core library
cargo clippy -p ramus-core                  # Lint
```

### Tauri App
```sh
cargo tauri dev                             # Dev mode (Rust + React hot-reload)
cargo tauri build                           # Production build
cargo test -p ramus-tauri                   # Tauri command tests
```

### React Frontend
```sh
cd ui && npm install                        # Install dependencies
cd ui && npm run dev                        # Vite dev server
cd ui && npm run build                      # Production build (tsc + vite)
```

## Key Design Decisions

- **Phases 1-11 are pure Rust** with comprehensive unit tests. No frontend work until the Rust core is fully tested. Each phase produces a testable module — never proceed with failing tests.
- **rusqlite** (not GRDB) for SQLite. WAL mode, FTS5 for track search, `parking_lot::Mutex` for connection safety.
- **libmpv via FFI** — evaluate `libmpv2` crate first, but raw `libmpv-sys` may be needed for playlist management and `af` filter support. The Swift `MPVController` is ~440 lines of direct C API calls.
- **Token encryption** uses AES-256-GCM keyed to platform hardware UUID (IOKit on macOS, registry on Windows, `/etc/machine-id` on Linux).
- **Plex API durations are in milliseconds** — convert to seconds at the boundary.
- **Search operator syntax** (`/genre`, `@artist`, `!track`, `%album`, `year:>2000`, `fav:`) segments on `" AND "` (case-sensitive uppercase). Without AND, entire input belongs to the first operator.
- **FTS5 escaping**: strip `"*():^{}`, replace `-` with space (FTS5 NOT operator).
- **Genre tree** uses Wikidata hierarchy (`open.json`, ~40k lines). Fuzzy matching (threshold 0.4) handles Plex genre name variations. Match results are cached.
- **EQ filter string** must use POSIX locale for float formatting — comma decimal separators break lavfi. Clear filters with `af=no` (not empty string).
- **Transcode endpoint** is `/music/:/transcode/universal/start.m3u8` — NOT `/audio/:/transcode/...`. Requires `X-Plex-Platform: Chrome` header.
- **`loadfile "replace"` implicitly stops** — do NOT call `mpv.stop()` before `load_queue`, it races with playlist setup.
- **Session reporting**: always send `state=stopped` for the previous track before reporting a new one. Scrobble at >= 90% progress, once per track.

## Swift Reference Mapping

When implementing a Rust module, consult the corresponding Swift source:

| Rust Module | Swift Reference |
|---|---|
| `models.rs` | `RamusMusicCore/Sources/Models/Types.swift`, `PlaybackConfig.swift` |
| `plex/client.rs` | `RamusMusicCore/Sources/PlexAPI/PlexClient.swift` |
| `plex/auth.rs` | `RamusMusicCore/Sources/PlexAPI/PlexAuth.swift` |
| `plex/token_store.rs` | `RamusMusicCore/Sources/PlexAPI/TokenStore.swift` |
| `plex/connection.rs` | `RamusMusicCore/Sources/PlexAPI/ConnectionMonitor.swift` |
| `playback/mpv.rs` | `RamusMusicCore/Sources/Playback/MPVController.swift` |
| `playback/player.rs` | `RamusMusicCore/Sources/Playback/AudioPlayer.swift` |
| `playback/session.rs` | `RamusMusicCore/Sources/Playback/SessionReporter.swift` |
| `playback/transcode.rs` | `RamusMusicCore/Sources/Playback/TranscodeHelper.swift` |
| `playback/lyrics.rs` | `RamusMusicCore/Sources/Playback/LyricsProvider.swift` |
| `playback/waveform.rs` | `RamusMusicCore/Sources/Playback/WaveformProcessor.swift` |
| `cache/db.rs` | `RamusMusicCore/Sources/Cache/CacheDatabase*.swift` (4 files) |
| `cache/sync.rs` | `RamusMusicCore/Sources/Cache/SyncEngine.swift` |
| `search/parser.rs` | `RamusMusicCore/Sources/Search/QueryParser.swift` |
| `search/engine.rs` | `RamusMusicCore/Sources/Search/SearchEngine.swift` |
| `genre/node.rs` | `RamusMusicCore/Sources/GenreTree/GenreNode.swift` |
| `genre/mapper.rs` | `RamusMusicCore/Sources/GenreTree/GenreMapper.swift` |
| `genre/parser.rs` | `RamusMusicCore/Sources/GenreTree/CustomGenreParser.swift` |

Swift test files live under `swift-project/RamusMusicCore/Tests/` — port these to Rust (122+ tests specified in Appendix D of IMPLEMENTATION_PLAN.md).

## Sync Engine Details

The sync has 4 phases: Artists (type=8) → Albums (type=9) → Tracks (type=10) → Deep Genre Fetch. Three modes:
- **Full sync**: all items + deep genre fetch for all albums
- **Incremental sync**: changed items only (compare `updatedAt`), deep fetch only changed albums. Genre change detection also checks if the first genre tag differs (genre-only edits don't always bump `updatedAt`).
- **Genre sync**: skip items, deep fetch all albums

Deep genre fetch uses bounded concurrency (8 concurrent via `tokio::sync::Semaphore`). Batch upserts in groups of 500.

## Database Schema

Full schema is in Appendix A of `IMPLEMENTATION_PLAN.md`. Tables: `artists`, `albums`, `tracks`, `tracks_fts` (FTS5), `genres` (NOCASE), `album_genres` (junction). Album upserts use `COALESCE` to preserve existing rating/studio/colors that may only come from deep metadata fetches.

## Tauri IPC

Events flow Rust → frontend via `app.emit()`. Commands flow frontend → Rust via `#[tauri::command]`. `AppState` holds `Arc`-wrapped shared state (`PlexClient`, `CacheDatabase`, `AudioPlayer`, `GenreMapper`, `SearchEngine`, `SyncEngine`, `SessionReporter`, `ConnectionMonitor`, `Settings`).

## Frontend Patterns

- **Zustand** for state management (library, playback, settings stores)
- **@tanstack/react-virtual** for virtualised lists (genre tree has 6,250+ nodes)
- **Canvas-based** waveform rendering with quad-curve smoothing
- Dark theme enforced, CSS variables for dynamic accent color (`--accent-r/g/b`)
- Custom window chrome (`decorations: false`, CSS drag region)
- 150ms debounce on search input, 50ms debounce on EQ slider changes
