# Third-Party Notices

ramus's own source code is distributed under the MIT License (see
`LICENSE` at the repository root). It incorporates the following
third-party components. Some releases bundle GPL builds of libmpv and
FFmpeg; those releases are distributed as a whole under the GNU GPL
version 3 or later (see "Native libraries" below).

## Music genre hierarchy data (`ramus-tauri/data/open.json`)

The genre tree bundled with ramus was initially based on the
[beets](https://github.com/beetbox/beets) project's
`beetsplug/lastgenre/genres-tree.yaml`. It has since been substantially
extended and restructured: many genres have been added, the hierarchy
has been reorganised (with some genres appearing under multiple parents),
and a large set of aliases (AKAs) is layered on top via
`ramus-tauri/data/aka.txt`. The current tree is roughly twice the size
of the original beets source.

- Original beets data: Copyright (c) 2010-2016 Adrian Sampson
- Licensed under the MIT License

The original beets hierarchy was compiled primarily from Wikipedia;
Wikipedia text content is available under CC BY-SA 3.0.

## Native libraries: libmpv, FFmpeg and their dependencies

ramus plays audio through libmpv (https://github.com/mpv-player/mpv) on
every platform, and each release ships a libmpv build together with
FFmpeg and their supporting libraries. Whether that build is GPL or LGPL
depends on where it comes from:

| Release | libmpv build | Licence |
| ------- | ------------ | ------- |
| macOS `.dmg` | Homebrew's `mpv`, `ffmpeg` and their dependencies, bundled by `scripts/bundle-macos-libmpv.py` | GPL-3.0-or-later as a whole: FFmpeg is configured with `--enable-gpl --enable-version3`, mpv with its `gpl` feature, and x264, x265 and rubberband are GPL |
| Linux AppImage | Ubuntu 24.04's `libmpv2` and its dependencies, bundled by `scripts/bundle-linux-libmpv.py` | GPL-3.0-or-later as a whole: FFmpeg is configured with `--enable-gpl`, libmpv links libcdio (GPL-3.0-or-later), and x264, x265 and xvid are GPL |
| Linux `.deb` / `.rpm` | the distribution's own libmpv package, installed as a dependency | not shipped by ramus |
| Windows installers | `libmpv-2.dll` from the LGPL build of [zhongfly/mpv-winbuild](https://github.com/zhongfly/mpv-winbuild), mirrored in this repository's `libmpv-windows-*` pre-release and pinned by SHA-256 in `.github/workflows/release.yml` | LGPL: mpv is built with `-Dgpl=false` (LGPL-2.1-or-later) and FFmpeg without `--enable-gpl` but with `--enable-version3` (LGPL-3.0-or-later) |
| iOS `.ipa` | [MPVKit-lavfi](https://github.com/1337raspberry/MPVKit-lavfi), statically linked | LGPL; the components are listed below |
| Android `.apk` | the [`dev.jdtech.mpv:libmpv`](https://github.com/jarnedemeulemeester/libmpv-android) AAR | GPL-3.0-or-later as a whole: FFmpeg is configured with `--enable-gpl --enable-version3` and mpv with its `gpl` feature |

A release that bundles a GPL build is distributed as a whole under the
GNU GPL version 3 or later (`licenses/LICENSE.GPL-3.0`); ramus's own MIT
code is GPL-compatible and keeps its MIT licence. The LGPL texts are at
`licenses/LICENSE.LGPL-2.1` and `licenses/LICENSE.LGPL-3.0`. The
corresponding source for ramus itself is the matching release tag at
https://github.com/1337raspberry/ramus; the source for each bundled
library is listed below or in the generated manifest.

### macOS and the Linux AppImage

The bundled set comes from the package manager on the build machine and
changes from build to build, so the bundle scripts record it at build
time. Each macOS app and AppImage carries
`licenses/native/NATIVE_LIBRARIES.md`, listing every bundled library file
with its package, version, licence and a link to its source, and beside
it the licence or copyright file of each package.

### Windows

`libmpv-2.dll` is a single library with FFmpeg and libmpv's other
dependencies linked in statically. The package list and build scripts
are in [zhongfly/mpv-winbuild](https://github.com/zhongfly/mpv-winbuild)
(its `compile-lgpl-libmpv.patch` produces the LGPL build) and the
[mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake)
tree it patches.

### Replacing libmpv

On desktop and Android libmpv is loaded dynamically, and the user may
substitute their own copy. On iOS it is statically linked, together with
the libraries listed below; ramus's own source is published under the MIT
License, so the app can be rebuilt and relinked against a modified
library.

- **Desktop** (macOS / Windows / Linux) — place an alternative `libmpv`
  on the dynamic library search path. See `ramus-tauri/src/mpv_ffi.rs`
  for the platform-specific search paths.
- **iOS** — libmpv is provided by the
  [MPVKit-lavfi](https://github.com/1337raspberry/MPVKit-lavfi) Swift
  Package, resolved by Xcode at build time: an LGPL build of
  [MPVKit](https://github.com/mpvkit/MPVKit) with a few more FFmpeg audio
  filters enabled. Its build scripts are published in that repository.
  To relink, point the package pin in `ramus-tauri/gen/apple/project.yml`
  and `plugins/tauri-plugin-ramus-ios-bridge/ios/Package.swift` at a
  modified build and rebuild the app (see the README's iOS build steps).
- **Android** — libmpv is provided by the
  [`dev.jdtech.mpv:libmpv`](https://github.com/jarnedemeulemeester/libmpv-android)
  Maven Central AAR (`v1.0.0` at the time of writing). The AAR ships the
  `.so` files for all four Android ABIs; users may rebuild the AAR with
  a different libmpv build and substitute it via Gradle.

### Other native libraries bundled in the Android AAR

`dev.jdtech.mpv:libmpv:1.0.0` packages a complete libmpv build along
with the supporting libraries it depends on. They are dynamically linked
inside the AAR's `.so` files and ship in every Android release. Each
retains its own upstream license:

| Library     | Version  | License                            | Upstream                                                   |
| ----------- | -------- | ---------------------------------- | ---------------------------------------------------------- |
| mpv (libmpv)| 0.41.0   | GPL-2.0-or-later (`gpl` feature)   | https://github.com/mpv-player/mpv                          |
| ffmpeg      | 8.1      | GPL-3.0-or-later (`--enable-gpl --enable-version3`) | https://ffmpeg.org/                        |
| libplacebo  | 7.360.1  | LGPL-2.1-or-later                  | https://code.videolan.org/videolan/libplacebo              |
| fribidi     | 1.0.16   | LGPL-2.1-or-later                  | https://github.com/fribidi/fribidi                         |
| libunibreak | 6.1      | LGPL-2.1-or-later / Apache-2.0     | https://github.com/adah1972/libunibreak                    |
| libass      | 0.17.4   | ISC                                | https://github.com/libass/libass                           |
| harfbuzz    | 14.1.0   | Old MIT                            | https://github.com/harfbuzz/harfbuzz                       |
| freetype    | 2.14.3   | FTL or GPL-2.0                     | https://gitlab.freedesktop.org/freetype/freetype           |
| fontconfig  | 2.17.1   | fontconfig (MIT-style)             | https://gitlab.freedesktop.org/fontconfig/fontconfig       |
| mbedtls     | 3.6.6    | Apache-2.0                         | https://github.com/Mbed-TLS/mbedtls                        |
| dav1d       | 1.5.3    | BSD-2-Clause                       | https://code.videolan.org/videolan/dav1d                   |
| libxml2     | 2.15.2   | MIT                                | https://gitlab.gnome.org/GNOME/libxml2                     |
| lua         | 5.2.4    | MIT                                | https://www.lua.org/                                       |

Because FFmpeg and mpv are GPL builds here, the Android APK as a whole
is distributed under GPL-3.0-or-later (`licenses/LICENSE.GPL-3.0`); the
LGPL-2.1-or-later components keep their licence, whose text is at
`licenses/LICENSE.LGPL-2.1`. Source code is available at each upstream
listed above. The libmpv-android packaging itself is the work of
[jarnedemeulemeester](https://github.com/jarnedemeulemeester/libmpv-android);
the exact build configuration used for each AAR version, which
reproduces the bundled libraries, is in that repository at the matching
tag (`v1.0.0`).

### Native libraries statically linked into the iOS app

`MPVKit-lavfi` 1.0.0 provides libmpv as static libraries along with the
libraries it depends on, all of which are linked into the iOS app. Each
retains its own upstream license:

| Library          | Version  | License                                    | Upstream                                              |
| ---------------- | -------- | ------------------------------------------ | ----------------------------------------------------- |
| mpv (libmpv)     | 0.41.0   | LGPL-2.1-or-later                          | https://github.com/mpv-player/mpv                     |
| ffmpeg           | 8.1.2    | LGPL-2.1-or-later (non-GPL build)          | https://ffmpeg.org/                                    |
| libplacebo       | 7.360    | LGPL-2.1-or-later                          | https://code.videolan.org/videolan/libplacebo         |
| libbluray        | 1.4.0    | LGPL-2.1-or-later                          | https://code.videolan.org/videolan/libbluray          |
| fribidi          | 1.0.16   | LGPL-2.1-or-later                          | https://github.com/fribidi/fribidi                    |
| gnutls           | 3.8.11   | LGPL-2.1-or-later                          | https://gnutls.org/                                    |
| nettle, hogweed  | 3.10     | LGPL-3.0-or-later or GPL-2.0-or-later      | https://www.lysator.liu.se/~nisse/nettle/             |
| gmp              | 6.2.1    | LGPL-3.0-or-later or GPL-2.0-or-later      | https://gmplib.org/                                    |
| uchardet         | 0.0.8    | MPL-1.1 or GPL-2.0-or-later or LGPL-2.1-or-later | https://www.freedesktop.org/wiki/Software/uchardet/ |
| libass           | 0.17.5   | ISC                                        | https://github.com/libass/libass                      |
| harfbuzz         | 14.2.0   | Old MIT                                    | https://github.com/harfbuzz/harfbuzz                  |
| freetype         | 2.14.3   | FTL or GPL-2.0                             | https://gitlab.freedesktop.org/freetype/freetype      |
| libunibreak      | 6.1      | Zlib                                       | https://github.com/adah1972/libunibreak               |
| OpenSSL          | 3.3.5    | Apache-2.0                                 | https://www.openssl.org/                               |
| MoltenVK         | 1.4.2    | Apache-2.0                                 | https://github.com/KhronosGroup/MoltenVK              |
| shaderc          | 2025.5.0 | Apache-2.0                                 | https://github.com/google/shaderc                     |
| lcms2            | 2.17     | MIT                                        | https://github.com/mm2/Little-CMS                     |
| libdovi          | 3.3.2    | MIT                                        | https://github.com/quietvoid/dovi_tool                |
| dav1d            | 1.5.3    | BSD-2-Clause                               | https://code.videolan.org/videolan/dav1d              |
| uavs3d           | 1.2.1    | BSD-3-Clause                               | https://github.com/uavs3/uavs3d                       |

For the LGPL-2.1-or-later components the license text at
`licenses/LICENSE.LGPL-2.1` applies. nettle, hogweed and gmp are used
under LGPL-3.0-or-later, whose text is at `licenses/LICENSE.LGPL-3.0`;
the LGPL-3.0 is a set of additional permissions on top of the GNU GPL
version 3, whose text is at `licenses/LICENSE.GPL-3.0`. Both ship in
every ramus release. Source code for each component is available from
the upstream listed above, and the exact build configuration from the
MPVKit-lavfi repository.

## Bundled fonts

Three font files are bundled in the frontend (`ui/src/fonts/`) and
loaded by the renderer:

- **Inter** (`InterVariable.ttf`) — Copyright (c) The Inter Project
  Authors (https://github.com/rsms/inter). Licensed under the
  [SIL Open Font License 1.1](https://openfontlicense.org/).
- **JetBrains Mono** (`JetBrainsMono-Variable.ttf`) — Copyright (c)
  JetBrains s.r.o. Licensed under the
  [SIL Open Font License 1.1](https://openfontlicense.org/). Used for
  the monospace UI surfaces (debug panel, technical detail rows).
- **Twemoji Country Flags** (`TwemojiCountryFlags.woff2`) — built from
  the [country-flag-emoji-polyfill](https://github.com/talkjs/country-flag-emoji-polyfill)
  project (MIT) and Twitter's Twemoji glyphs, which are licensed under
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Used to
  render flag emoji on platforms whose system fonts don't include them.

## Mozilla Public License 2.0 components

A few bundled Rust crates are distributed under MPL-2.0: the
Servo-heritage CSS crates (`cssparser`, `selectors` and their helpers)
that Tauri's webview layer depends on, and `option-ext`, reached through
the `directories` crate. `THIRD_PARTY_LICENSES.md` beside this file lists
every crate with its licence. MPL-2.0 is a file-scope copyleft — it
applies only to the MPL-licensed source files themselves and does not
affect the rest of ramus.

- License text: `licenses/LICENSE.MPL-2.0` in the installed application,
  or https://www.mozilla.org/media/MPL/2.0/index.txt
- Full crate list: see `THIRD_PARTY_LICENSES.md`

## All other third-party software

See `THIRD_PARTY_LICENSES.md` for the full list of bundled Rust crates
and npm packages together with their license text. That file is
generated by `scripts/generate-third-party-licenses.py` from `Cargo.lock`
and `ui/pnpm-lock.yaml`; CI fails on drift.
