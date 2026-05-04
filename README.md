<div align="center">

<img title="" src="ramuslogo.png" alt="ramus" width="160" height="160" data-align="center">

<h1 align="center">ramus</h1>
<p align="center"><sub>ramus | ra·​mus | a projecting part, elongated process, or branch</sub></p>

<!-- TAGLINE: one-line pitch goes here -->

[![CI](https://github.com/1337raspberry/ramus/actions/workflows/ci.yml/badge.svg)](https://github.com/1337raspberry/ramus/actions/workflows/ci.yml)
[![Release](https://github.com/1337raspberry/ramus/actions/workflows/release.yml/badge.svg)](https://github.com/1337raspberry/ramus/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/1337raspberry/ramus?display_name=tag&include_prereleases)](https://github.com/1337raspberry/ramus/releases)
[![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux%20%7C%20iOS%20%7C%20Android-lightgrey)](#download)

</div>

<!-- HERO SCREENSHOT: drop a wide screenshot of the main library/now-playing view here -->

<p align="center">
  <img src="docs/screenshots/hero.png" alt="ramus main view" width="900" />
</p>

---

## About

<!--
  WRITE-UP GOES HERE — what is ramus, who is it for, the elevator pitch,
  the bit about why it exists. A couple of paragraphs is plenty.
-->

> genre first music client for Plex focused on discoverability and exploring your library

## Features

<!--
  FEATURE GRID — pick the highlights and write them with personality.
  Suggested shape below; replace freely.
-->

|         |                                                           |
| ------- | --------------------------------------------------------- |
| _one_   | _genre tree view blah blah_                               |
| _two_   | _fuzzy search_                                            |
| _three_ | _bookmarks and filters_                                   |
| _four_  | _album art focused with themed accents etc_               |
| _five_  | _waveform seeking_                                        |
| _six_   | _desktop visualiser_                                      |
| seven   | _hot tracks + popularity charts_                          |
| _eight_ | _offline sync with no limits. download 100gb if you want_ |

> _placeholder for the deeper feature copy / bulleted highlights. inb4 a billion words_

## Screenshots

<!-- Drop screenshots into docs/screenshots/ and reference them here. -->

<table>
  <tr>
    <td><img src="docs/screenshots/library.png" alt="Library view" /></td>
    <td><img src="docs/screenshots/nowplaying.png" alt="Now playing" /></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/search.png" alt="Search with operators" /></td>
    <td><img src="docs/screenshots/focus.png" alt="Focus / spectrum view" /></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/mobile.png" alt="iOS/Android" /></td>
    <td><img src="docs/screenshots/filters.png" alt="filters" /></td>
  </tr>
</table>

---

## Download

Pre-built installers are produced by [GitHub Actions](https://github.com/1337raspberry/ramus/actions/workflows/release.yml) and attached to each [Release](https://github.com/1337raspberry/ramus/releases).

| Platform                          | Artifact                                           | Notes                                                                                                  |
| --------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **macOS** (Apple Silicon + Intel) | `ramus_<version>_universal.dmg`                    | Universal binary. libmpv + ffmpeg/codec stack bundled inside the `.app`.                               |
| **Windows 10/11 (x64)**           | `ramus_<version>_x64-setup.exe` (NSIS) or `ramus_<version>_x64_en-US.msi` | `libmpv-2.dll` ships next to the executable.                                                           |
| **Linux (x86_64)**                | `ramus_<version>_amd64.AppImage` / `.deb` / `.rpm` | The AppImage bundles libmpv. The `.deb` / `.rpm` depend on the system `libmpv2` / `mpv-libs` package.  |
| **iOS**                           | Build from source only for now                     | More on that below.                                                                                    |
| **Android**                       | `ramus_<version>_universal.apk`                    | Signed multi-ABI APK (`arm64-v8a` + `armeabi-v7a`); sideload via `adb install` or your file manager.   |

> ⚠️ **Desktop releases are unsigned today.** macOS will quarantine the `.app`; Windows SmartScreen will warn. The release notes include the standard `xattr -cr ramus.app` and SmartScreen "More info → Run anyway" workarounds. If that's a dealbreaker, build from source.

### Requirements

- **macOS** — the bundle declares a theoretical minimum of **10.13 (High Sierra)**, but the only versions I've actually run it on are **macOS 15 (Sequoia)** and newer. Anything older may work, may not — no promises.
- **Windows** 10 (x64) or newer; WebView2 runtime (preinstalled on Windows 11; the installer pulls it in on Windows 10).
- **Linux** with WebKitGTK 4.1 (most current distributions). The `.deb` / `.rpm` need `libmpv2` (or `mpv-libs`) installed; the AppImage doesn't.
- **iOS** 17.5 or newer.
- **Android** 7.0 Nougat (API 24) or newer.
- A **Plex Media Server** you can sign into. Music libraries only — ramus is audio-focused.

---

## Build from source

You'll need:

- **Rust** stable (`rustup install stable`).
- **Node.js** 20+ and **npm**.
- **Tauri 2 prerequisites** for your platform — follow [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/).
- **libmpv** in the dev loop:
  - macOS: `brew install mpv` (provides `libmpv.dylib`).
  - Linux (Debian/Ubuntu): `libmpv-dev libwebkit2gtk-4.1-dev libgtk-3-dev` (and `build-essential pkg-config` if you don't have them already).
  - Linux (Fedora): `mpv-devel webkit2gtk4.1-devel gtk3-devel`.
  - Windows: the build script downloads a prebuilt LGPL DLL; nothing to install manually.
- **For mobile**:
  - **iOS — building**: Xcode + `xcodegen` + `cocoapods`. That's enough for `cargo tauri ios build`.
  - **iOS — deploying to a tethered device** (`cargo tauri ios dev`): also `libimobiledevice` + `ios-deploy` (`brew install libimobiledevice ios-deploy`).
  - **Android**: Android Studio + the NDK installed via SDK Manager.

```sh
git clone https://github.com/1337raspberry/ramus
cd ramus

# Frontend deps
( cd ui && npm install )

# Run the desktop app (Rust + React hot-reload)
cargo tauri dev

# Build a release artifact for your current platform
cargo tauri build
```

Mobile:

```sh
# iOS
./scripts/regen-ios-project.sh
cargo tauri ios build
```

> You'll need to sideload this yourself. If demand is there I may put this on the App Store, but I'm not in a rush to pay Apple for the privilege of releasing my open-source, entirely free app.

```sh
# Android — emulator or connected device
cargo tauri android build
```

Pre-flight checks (CI runs the same):

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ramus-core
( cd ui && npx tsc --noEmit )
```

---

## Architecture

```
ramus-core/    Rust library — all business logic.
               Plex client, cache (rusqlite + FTS5), sync engine,
               search, genre tree, playback core.
ramus-tauri/   Tauri 2 app shell.
               IPC commands, libmpv FFI, media controls,
               iOS Swift bridge, Android Kotlin bridge.
ui/            React + Vite + Zustand + TypeScript.
plugins/       Tauri plugin: iOS Swift bridge (MPVKit) and
               Android Kotlin bridge (Media3 / ExoPlayer).
scripts/       Build helpers (libmpv bundling, codesigning,
               license regeneration).
```

### Tech stack

- **Backend** — [Rust](https://www.rust-lang.org/), [Tauri 2](https://tauri.app/), [rusqlite](https://github.com/rusqlite/rusqlite) with WAL + FTS5, [reqwest](https://github.com/seanmonstar/reqwest), [tokio](https://tokio.rs/).
- **Audio** — [libmpv](https://mpv.io/) (loaded dynamically via [libloading](https://github.com/nagisa/rust_libloading)) on desktop; [MPVKit](https://github.com/mpvkit/MPVKit) on iOS; [Media3 / ExoPlayer](https://developer.android.com/media/media3) on Android.
- **DSP** — [symphonia](https://github.com/pdeljanov/Symphonia) + [rustfft](https://github.com/ejmahler/RustFFT) for the per-track spectrum analyser.
- **System integration** — [souvlaki](https://github.com/Sinono3/souvlaki) for desktop media keys / Now Playing; `MPRemoteCommandCenter` on iOS; `MediaSession` + `MediaSessionService` on Android.
- **Frontend** — [React 19](https://react.dev/) + [Vite](https://vite.dev/) + [TypeScript](https://www.typescriptlang.org/), [Zustand](https://github.com/pmndrs/zustand) for state, [@tanstack/react-virtual](https://tanstack.com/virtual) for the long lists.
- **Fonts** — [Inter](https://rsms.me/inter/), [JetBrains Mono](https://www.jetbrains.com/lp/mono/), [Twemoji Country Flags](https://github.com/talkjs/country-flag-emoji-polyfill).

Most behaviour lives in `ramus-core` and is unit-tested. The Tauri layer is intentionally thin — IPC plumbing on top of the core, plus the platform-specific glue (FFI on desktop, Swift/Kotlin bridges on mobile).

---

## Privacy

- **Plex auth tokens** are encrypted at rest with **AES-256-GCM**, using a key derived (`SHA-256`) from a stable per-machine identifier:
  
  - **macOS** — `IOPlatformUUID` (read via IOKit).
  - **Windows** — `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`.
  - **Linux** — `/etc/machine-id`.
  - **Android** — a random UUID generated on first run and stored in the app's sandboxed config dir.
  - **iOS** is the exception: tokens go directly into the system Keychain (no file, no AES layer), via a Swift `KeychainBridge`.
  
  The encrypted blob (`tokens.enc`) lives in the platform's standard app-data directory and is written atomically (tmp + `fsync` + `rename`) with `0o600` permissions on Unix. The threat model is "render the file inert if exfiltrated to another machine" - not protection against a local attacker who already has root on the same device. This same auth token is not even encrypted at all in many other clients.

- **No telemetry, no analytics, no crash reporters, no auto-updaters. Not ever.** ramus only talks to your Plex Media Server and plex.tv (for OAuth + server discovery). No other connections anywhere.

- **Server auth tokens are kept out of logs and UI.** Plex authenticates by query string (`?X-Plex-Token=…`), and reqwest's error formatter includes the failing URL. ramus redacts both: track URLs are never logged in full (only `ratingKey` / part-key), and `prefetch.rs::redact_reqwest_err()` peels the underlying error so download failures don't leak the token in logs. The mobile debug panel (long-press the EQ button) masks `X-Plex-Token=` and `X-Plex-Headers=` before rendering anything.

- **Local databases** (cache, image cache, audio cache, downloads) live in your platform's standard app-data directory:
  
  - macOS: `~/Library/Application Support/com.raspsoft.ramus/` (desktop). On iOS, everything lives inside the app's sandboxed container.
  - Windows: `%APPDATA%\raspsoft\ramus\`.
  - Linux: `~/.local/share/ramus/` (XDG-respecting `$XDG_DATA_HOME`).
  - Android: app-private storage; `network_security_config.xml` permits cleartext HTTP for LAN Plex servers (so you can reach `http://192.168.x.x:32400`) and nothing else.

## Security

If you've found a vulnerability, please **don't open a public issue**. See [SECURITY.md](SECURITY.md) for the disclosure process and scope. In short: report through [GitHub Security Advisories](https://github.com/1337raspberry/ramus/security/advisories/new). 

---

## Contributing

Bug reports, feature requests, and PRs are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before sending a non-trivial PR — it covers the project layout, the pre-push checks, and a few of the load-bearing constraints around playback timing and the mobile bridges.

This project follows the [Contributor Covenant 2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).

---

## Limitations, Known Issues & Planned Improvements

- transcode streams are a bit janky on mobile or non stable connections. I want to build an HLS + prefetch combo logic with smarter connection monitoring and lifecycle but that will take some time and dedicated testing. Currently, this is the biggest caveat and I'd call the mobile apps still Beta at best due to this. It's going to be the main focus of my development for now.

- visualiser requires dedicated processing, actual fft cant be read in-line with current mpv stack. This is of course not ideal. It also requires a second initial download of the first track because stream-download cant be used for this purpose, which kind of sucks. Feature is just for looks though and defaults to off

- the pre-sync concept and implementation in general - required for fuzzy search, and also because plex only serves 2 genres/styles via its standard calls, you need to do a deep api call per album to get more than 2. This is simply a plex limitation and unless they change their api, we're not getting around it. we could refactor a lot and get rid of this but I think the trade-off is worth it. Even on massive libraries, remotely, it's only a few minutes for very first sync, and it's what the whole app is really about. 

- no playlist support. On the fence if i even want to add this. We have pseudo playlist conceptually via bookmarks and filters, but we could do actual playlists too if we want.

- I wouldn't mind improving some of the more hidden/unintuitive ux. I've spent a lot of time making it as obvious and, what i think is user friendly as possible. But this is still very much a myopic one person project. What is obvious to me might not be obvious to everybody else.

- the colour extraction and accent colours still aren't as perfect as I'd like, and there are some edge cases where things aren't as readable as I'd want, but I've done loads of tweaking to try and get to a happy medium, on many different displays, SDR and HDR too. So I might just have to accept that perfect is the enemy of good, or whatever they say.

- I wouldn't mind implementing the new JWT short lived token auth system that plex has recently rolled out, but as far as I can tell it only applies to plex.tv auth, not PMS server auth, so that token is always going to be perma and long standing. Mixing the two isn't ideal, so when that is fully baked into PMS, i would like to roll that out. Again though, we're defending against an absolute worst case scenario when we talk about our auth tokens and i must point out that even official plex clients, store their auth tokens in plain text, so it's clearly an "accepted" risk in the plex ecosystem.

---

## Trademarks & affiliation

ramus is an **independent third-party client**. It is not affiliated with, endorsed by, or sponsored by Plex or any of the upstream projects it builds on.

---

## Custom Hierarchy Tool

- details of how this works goes here. also i gotta setup an import feature on that too so people can import then customise then export again as .txt for sharing etc.

---

## License

ramus is licensed under the [MIT License](LICENSE).

### Third-party software

ramus links — at runtime, dynamically — against **libmpv** ([LGPL-2.1-or-later](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html)) on desktop. libmpv source is available at [github.com/mpv-player/mpv](https://github.com/mpv-player/mpv); a copy is shipped under `licenses/` in every release artifact. You may substitute your own libmpv build by placing it on the dynamic library search path — the search paths are documented in [`ramus-tauri/src/mpv_ffi.rs`](ramus-tauri/src/mpv_ffi.rs).

A handful of bundled Rust crates (notably the [symphonia](https://github.com/pdeljanov/Symphonia) audio decoder family) are distributed under [MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/). MPL-2.0 is file-scope copyleft and does not affect the rest of ramus.

The bundled music genre tree (`ramus-tauri/data/open.json`) is derived from the [beets](https://github.com/beetbox/beets) project's `genres-tree.yaml` (MIT, © Adrian Sampson) and has been substantially extended. See [NOTICE.md](NOTICE.md).

The full list of third-party Rust crates and npm packages — together with their license texts — is in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md), regenerated from `Cargo.lock` and `ui/package-lock.json` by [`scripts/generate-third-party-licenses.py`](scripts/generate-third-party-licenses.py).

## Acknowledgements

ramus is proudly built on the open source wonders that are:

- [mpv](https://mpv.io/) and the libmpv contributors.
- [Tauri](https://tauri.app/) - web frontend with minimal bloat, glorious
- [Plex](https://www.plex.tv/) - for the media server that makes this app possible.
- [beets](https://github.com/beetbox/beets) - for seeding the genre hierarchy (and helping me tag my music every day!)
- The [symphonia](https://github.com/pdeljanov/Symphonia), [rustfft](https://github.com/ejmahler/RustFFT), [souvlaki](https://github.com/Sinono3/souvlaki), and [rusqlite](https://github.com/rusqlite/rusqlite) maintainers — and everyone else listed in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
