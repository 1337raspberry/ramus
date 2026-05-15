# Third-Party Notices

ramus is distributed under the MIT License (see `LICENSE` at the
repository root). It incorporates the following third-party components.

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

## Runtime-linked native library: libmpv

ramus dynamically loads libmpv at runtime to provide audio playback on
every platform (desktop, iOS, Android). libmpv is distributed under
LGPL-2.1-or-later.

- Upstream: https://github.com/mpv-player/mpv
- License text: `licenses/LICENSE.LGPL-2.1` in the installed application,
  or https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt

Source code for libmpv can be obtained from https://github.com/mpv-player/mpv.

libmpv is loaded dynamically (not statically linked) on every platform,
and the user may substitute their own copy:

- **Desktop** (macOS / Windows / Linux) — place an alternative `libmpv`
  on the dynamic library search path. See `ramus-tauri/src/mpv_ffi.rs`
  for the platform-specific search paths.
- **iOS** — libmpv is provided by the [MPVKit](https://github.com/mpvkit/MPVKit)
  Swift Package, resolved by Xcode at build time.
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
| mpv (libmpv)| 0.41.0   | LGPL-2.1-or-later                  | https://github.com/mpv-player/mpv                          |
| ffmpeg      | 8.1      | LGPL-2.1-or-later (non-GPL build)  | https://ffmpeg.org/                                         |
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

For the LGPL-2.1-or-later components, source code is available at each
upstream listed above; the LGPL license text shipped at
`licenses/LICENSE.LGPL-2.1` (and bundled into every ramus release)
applies. The libmpv-android packaging itself is the work of
[jarnedemeulemeester](https://github.com/jarnedemeulemeester/libmpv-android);
the exact build configuration used for each AAR version is in that
repository.

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

Several bundled Rust crates are distributed under MPL-2.0, most notably
the `symphonia` audio decoder family used by the focus-mode spectrum
visualiser. MPL-2.0 is a file-scope copyleft — it applies only to the
MPL-licensed source files themselves and does not affect the rest of
ramus.

- License text: `licenses/LICENSE.MPL-2.0` in the installed application,
  or https://www.mozilla.org/media/MPL/2.0/index.txt
- Full crate list: see `THIRD_PARTY_LICENSES.md`

## All other third-party software

See `THIRD_PARTY_LICENSES.md` for the full list of bundled Rust crates
and npm packages together with their license text. That file is
generated by `scripts/generate-third-party-licenses.py` from `Cargo.lock`
and `ui/pnpm-lock.yaml`; CI fails on drift.
