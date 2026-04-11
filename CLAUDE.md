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
│       ├── settings.rs  # Settings I/O (load/save to disk)
│       ├── util.rs      # Shared utilities (FTS5/LIKE escaping, percent-encoding, codec checks)
│       ├── plex/        # HTTP client, OAuth, token store, connection monitor
│       ├── playback/    # mpv wrapper, audio player, queue, EQ, lyrics, waveform, session reporting
│       ├── cache/       # rusqlite SQLite (WAL, FTS5), sync engine, image cache
│       ├── search/      # Query parser (operator syntax), search engine
│       └── genre/       # Genre tree, fuzzy mapper, custom genre parser
├── ramus-tauri/         # Tauri 2 app shell
│   └── src/
│       ├── commands/    # Tauri IPC command handlers (auth, library, playback, search, settings, sync)
│       ├── events.rs    # Event emission helpers (playback-state, accent-color, sync-progress, etc.)
│       ├── state.rs     # AppState — Arc-wrapped shared state
│       ├── mpv_ffi.rs   # Raw libmpv C FFI bindings
│       ├── mpv_controller.rs  # High-level MpvPlayer implementation over FFI
│       ├── session_reporter.rs # Scrobble + timeline reporting orchestration
│       ├── prefetch.rs  # Background audio cache prefetch
│       └── auto_sync.rs # Background periodic sync scheduler
├── ui/                  # React frontend — Vite + TypeScript + Zustand
└── swift-project/       # Original Swift/SwiftUI reference implementation
```

## Build & Test Commands

### Rust Core
```sh
cargo test -p ramus-core                    # Run all core tests
cargo test -p ramus-core -- test_name       # Run a single test
cargo test -p ramus-core -- module::        # Run tests in a module
cargo build -p ramus-core                   # Build core library
cargo clippy -p ramus-core --all-targets    # Lint (production + tests)
```

### Tauri App
```sh
cargo tauri dev                             # Dev mode (Rust + React hot-reload)
cargo tauri build                           # Production build
cargo test -p ramus-tauri                   # Tauri command tests
```

### Before committing
CI runs clippy with `-D warnings` (warnings = errors) on **both** crates and **all targets** (lib + tests). A clean local lint requires the same flags — and per-crate clippy hides warnings in the other crate. Run this before every commit:
```sh
cargo clippy --workspace --all-targets -- -D warnings
```
Lints in test-only code (`#[cfg(test)]`) won't fail a normal `cargo build`/`cargo test`, but they **will** fail CI. Don't skip `--all-targets`.

### React Frontend
```sh
cd ui && npm install                        # Install dependencies
cd ui && npm run dev                        # Vite dev server
cd ui && npm run build                      # Production build (tsc + vite)
```

## Key Design Decisions

- **Phases 1-11 are pure Rust** with comprehensive unit tests. No frontend work until the Rust core is fully tested. Each phase produces a testable module — never proceed with failing tests.
- **rusqlite** (not GRDB) for SQLite. WAL mode, FTS5 for track search, `parking_lot::Mutex` for connection safety.
- **libmpv via runtime FFI** — loaded dynamically at startup through `libloading`, not statically linked. `mpv_ffi.rs` defines an `MpvLib` struct that holds the `Library` plus one cached function pointer per symbol; `MpvController` takes an `Arc<MpvLib>` so the same library is shared across the controller and its background event thread. **Drop order matters**: `_lib` is declared as the last field of `MpvLib` so it's dropped last — dropping it earlier would invalidate the function pointers. Don't reorder those fields. This approach means the app compiles on every platform without libmpv headers or import libs; `MpvLib::load()` searches `MPV_LIB_PATH`, paths next to the executable, and standard brew/apt locations, returning a multi-line error listing everything it tried.
- **Token encryption** uses AES-256-GCM keyed to platform hardware UUID (IOKit on macOS, registry on Windows, `/etc/machine-id` on Linux).
- **Plex API durations are in milliseconds** — convert to seconds at the boundary.
- **Search operator syntax** (`/genre`, `@artist`, `!track`, `%album`, `year:>2000`, `fav:`) segments on `" AND "` (case-sensitive uppercase). Without AND, entire input belongs to the first operator.
- **FTS5 escaping**: strip `"*():^{}`, replace `-` with space (FTS5 NOT operator).
- **Genre tree** uses beets MIT-licensed hierarchy (`open.json`, ~4k lines, 792 genres in 21 categories). A `GenreSource` setting toggles between the open-source tree and user-imported custom genres. Fuzzy matching handles Plex genre name variations. Match results are cached.
- **EQ filter string** must use POSIX locale for float formatting — comma decimal separators break lavfi. The `af` chain **always** includes the labelled `astats` metering filter alongside any EQ segment, so the focus-mode spectrum visualiser receives realtime peak/RMS data regardless of EQ state. Build the chain via `ramus_core::playback::player::build_af_string(enabled, bands)` — do not set raw strings or `af=no` (that would drop the astats label and break metering). The astats filter is also seeded via the `af` mpv option in `default_mpv_options()` so it runs from the first track.
- **Transcode endpoint** is `/music/:/transcode/universal/start.m3u8` — NOT `/audio/:/transcode/...`. Requires `X-Plex-Platform: Chrome` header.
- **`loadfile "replace"` implicitly stops** — do NOT call `mpv.stop()` before `load_queue`, it races with playlist setup.
- **Session reporting**: always send `state=stopped` for the previous track before reporting a new one. Scrobble at >= 90% progress, once per track.
- **macOS fullscreen** — `decorations: false` gives us a borderless NSWindow that macOS refuses to fullscreen. The `main.rs` setup hook flips `NSWindow.collectionBehavior |= NSWindowCollectionBehaviorFullScreenPrimary` (via `objc2::msg_send!`) to opt it in. Don't remove it — without it, the green traffic light, ⌃⌘F, and `setFullscreen(true)` are all no-ops.
- **macOS code signing gotcha (two parts, both load-bearing)** — macOS 26 kills ad-hoc-signed processes with `EXC_BAD_ACCESS (SIGKILL Code Signature Invalid) / CODESIGNING Invalid Page` the moment `dlopen` reads the first page of a dylib whose signature it doesn't accept. Two distinct causes trip this, both affecting libmpv loads:
  1. **`install_name_tool` corruption**: `scripts/bundle-macos-libmpv.py` rewrites the install ID and `@loader_path` deps on every bundled dylib. That invalidates the on-disk signature, and `install_name_tool`'s built-in ad-hoc re-sign is unreliable (quiet failures when there's no blob padding). The script **must** explicitly `codesign --force --sign -` + `codesign --verify --strict` every file afterwards — don't remove that pass. Release 0.8.0 shipped without it and every user hit the crash.
  2. **rustc/ld linker-signed flag**: rustc's default macOS linker ad-hoc signs binaries with the `linker-signed` flag (`flags=0x20002` instead of plain `0x2`). On macOS 26, linker-signed binaries running from **untrusted locations** — anything outside `/Applications/`, including `target/debug/` and `target/release/` — can't `dlopen` ad-hoc dylibs from brew paths. The same binary in `/Applications/` works fine, which is why only `cargo tauri dev` (and `cargo tauri build` outside CI) was affected once the bundler fix was in. `.cargo/config.toml` + `.cargo/macos-linker-wrapper.sh` fix this transparently: the wrapper invokes the real linker then runs `codesign --force --sign -` on the output, producing a plain ad-hoc signature the kernel accepts from any location. Both apply automatically to `aarch64-apple-darwin` and `x86_64-apple-darwin` — no manual step, including in CI for the universal release build. Don't reintroduce `linker-signed` via a custom `rustflags` linker override without also re-signing.
- **Windows asset-protocol scope gotcha** — the image cache writes to `{RoamingAppData}\raspsoft\ramus\data\image_cache\` because `ramus_core::plex::token_store::config_dir()` uses the `directories` crate (`ProjectDirs::from("com", "raspsoft", "ramus").data_dir()`). Tauri's `$APPDATA` scope variable resolves to `{RoamingAppData}\<bundle_identifier>\` = `…\com.ramus.app\`, which is a completely different subtree from `…\raspsoft\ramus\`. That's why the macOS (`$HOME/Library/Application Support/com.raspsoft.ramus/**`) and Linux (`$HOME/.local/share/ramus/**`) allow-entries exist — they target the `directories` crate's actual write path, not `$APPDATA`. The Windows entry was missed in 0.8.0 so album art wrote fine but every `<img>` read was silently blocked by the scope matcher; fix is `$HOME/AppData/Roaming/raspsoft/ramus/**` in `tauri.conf.json`. If the `directories`-crate config (org/app) ever changes, bump all three platform-specific entries together.

## Naming Conventions & Shared Utilities

### Canonical locations — do not duplicate

Shared utility functions live in `ramus-core/src/util.rs`. Before writing a helper, check if it already exists here:

| Function / Constant | Purpose |
|---|---|
| `escape_fts5(input)` | Strip FTS5 metacharacters for MATCH queries |
| `escape_like(input)` | Escape `%`, `_`, `\` for SQL LIKE patterns |
| `percent_encode(s)` | RFC 3986 percent-encoding |
| `percent_decode(s)` | Percent-decoding (`%2F` → `/`) |
| `LOSSLESS_CODECS` | `&["flac", "alac", "wav", "aiff", "aif", "pcm"]` |
| `is_lossless_codec(codec)` | Case-insensitive lossless check |

The Tauri command result type is defined once in `ramus-tauri/src/commands/mod.rs`:
```rust
pub type CmdResult<T> = Result<T, String>;
```
Command files import it via `use super::CmdResult;` — do not redefine locally.

Frontend format helpers live in `ui/src/lib/format.ts`:
- `formatDuration(seconds)` — `"m:ss"` display (do not create local copies or name variants like `formatTime`)
- `formatCodec(codec, bitrate)` — `"FLAC"` or `"MP3 320"` display

Color helpers live in `ui/src/lib/vibrantColor.ts`:
- `hexToRgb(hex)` — returns `[r, g, b]` (0-255), exported for shared use
- `accentFromPalette(p)`, `blurColorsFromPalette(p)`, `extractPalette(img)`

### Naming conventions by layer

| Layer | Convention | Examples |
|---|---|---|
| Rust structs/enums | PascalCase | `PlaybackStatus`, `CacheError` |
| Rust functions/fields | snake_case | `is_lossless_codec`, `rating_key` |
| Rust constants | UPPER_SNAKE_CASE | `LOSSLESS_CODECS`, `BATCH_SIZE` |
| SQLite columns | camelCase | `sourceId`, `artUrl`, `durationMs` |
| Tauri commands | snake_case | `get_genre_tree`, `play_tracks` |
| Tauri events | kebab-case | `playback-state`, `accent-color` |
| TS types/interfaces | PascalCase | `Album`, `PlaybackStatePayload` |
| TS functions | camelCase | `formatDuration`, `extractPalette` |
| TS constants | UPPER_SNAKE_CASE | `MIN_CARD_WIDTH`, `BAND_LABELS` |
| React components | PascalCase | `NowPlayingView`, `AlbumCard` |
| Zustand stores | `use[Name]Store` | `useLibraryStore`, `usePlaybackStore` |
| CSS classes | kebab-case with prefix | `.np-header`, `.album-card`, `.eq-panel` |
| CSS variables | `--kebab-case` | `--accent-r`, `--bg-primary` |

### Key type aliases

- `PlexID = String` — Plex `ratingKey`, used as the primary identifier for media items
- `Duration = f64` — always in **seconds** in Rust/TS; the DB stores `durationMs` (milliseconds), convert at boundary

### Rules to maintain consistency

- When adding a new utility that could be used across modules, put it in `util.rs` (Rust) or `ui/src/lib/` (TS) — never define helpers inline in component/command files
- The lossless codec list must include `"aif"` (valid AIFF extension) — always use `is_lossless_codec()` or import from `format.ts` rather than defining inline arrays
- Tauri event names are kebab-case strings; keep them in sync between `ramus-tauri/src/events.rs` (emit) and `ui/src/App.tsx` / stores (listen)
- DB column names are camelCase for Plex API compatibility — this is intentional, do not change to snake_case

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

Events flow Rust → frontend via `app.emit()`. Commands flow frontend → Rust via `#[tauri::command]`. `AppState` holds `Arc`-wrapped shared state (`PlexClient`, `CacheDatabase`, `AudioPlayer`, `GenreMapper`, `SearchEngine`, `SyncEngine`, `SessionReporter`, `ConnectionMonitor`, `Settings`, `ImageCache`, `reqwest::Client`, `discovered_servers`).

## Frontend Patterns

- **Zustand** for state management (library, playback, settings stores)
- **@tanstack/react-virtual** for virtualised lists (genre tree has 6,250+ nodes)
- **Canvas-based** waveform rendering with quad-curve smoothing
- Dark theme enforced, CSS variables for dynamic accent color (`--accent-r/g/b`)
- Custom window chrome (`decorations: false`, CSS drag region)
- 150ms debounce on search input, 50ms debounce on EQ slider changes
- **Favourite toggles** route through `libraryStore.toggleAlbumFav` / `toggleTrackFav`. These own the Tauri IPC call AND patch `playbackStore.nowPlayingAlbum` / `currentTrack` / `queue` when IDs match, keeping library views and the Now Playing card in sync. Do **not** call the `toggle_album_favourite` / `toggle_track_favourite` IPC commands directly from components — the UI won't re-render because the stores are the source of truth for star icons.
