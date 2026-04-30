# Contributing to ramus

Thanks for taking the time. ramus is a small project; the bar for contributions is to keep the codebase consistent and the user-visible behaviour correct.

## Ground rules

- Be kind. This project follows the [Contributor Covenant 2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
- Don't open a PR for security findings — use [GitHub Security Advisories](https://github.com/1337raspberry/ramus/security/advisories/new). See [SECURITY.md](SECURITY.md).
- For larger features (anything that touches sync, the database schema, or playback timing), open an issue first so we can sketch the approach before you write a thousand lines.

## Getting started

Build instructions per platform are in [README.md](README.md). If `cargo tauri dev` runs and you can sign into a Plex account, you're set.

## Project layout

```
ramus-core/    Rust library — all business logic. Plex client, cache (rusqlite + FTS5),
               sync engine, search, genre tree, playback core. Comprehensive unit tests.
ramus-tauri/   Tauri 2 app shell. IPC commands, mpv FFI, media controls, mobile bridges.
ui/            React + Vite + Zustand. Components, stores, mobile views.
plugins/       Tauri plugin: iOS Swift bridge (MPVKit) and Android Kotlin bridge (Media3).
scripts/       Build helpers (bundle-libmpv, codesign, regenerate licenses, AKA merge).
licenses/      Vendored license texts for runtime-linked native libraries (LGPL, MPL).
```

Most business-logic changes belong in `ramus-core` with tests. Keep the Tauri layer thin — it should be IPC plumbing, not logic.

## Before you push

CI runs the same checks. Save yourself the round-trip:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ramus-core
(cd ui && npx tsc --noEmit)
```

Per-crate clippy hides warnings in the other crate, and lints in `#[cfg(test)]` code don't fail `cargo build`. Use `--workspace --all-targets` or you'll find out from CI.

If you changed dependencies, regenerate the third-party license bundle (CI's drift check will catch you otherwise):

```sh
cargo install cargo-about --locked --version '^0.6'
python3 scripts/generate-third-party-licenses.py
```

## Code style

- **Comments** — write them for the next reader, not for the diff. Explain *why* something is non-obvious; don't restate the *what*. Skip the comment if a well-named identifier already says it.
- **No new abstractions** until you have three concrete call sites that need them. A bug fix doesn't need surrounding cleanup.
- **No half-finished implementations** in main. If a code path can't actually run yet, gate it or leave it out entirely.
- **Validate at boundaries, trust internal code.** Check user input and external API responses; don't pile on defensive checks for things internal callers already guarantee.

Rust is `rustfmt`-clean (no special config). TypeScript uses Prettier with the project's settings.

## Tests

- `ramus-core` has unit tests for everything non-trivial — search engine, genre mapper, Plex client error handling, cache schema, sync engine. Add tests in the same module.
- `ramus-tauri` is mostly integration glue; smoke-test by running the app.
- The frontend has no automated tests today. Manual smoke is enough for most PRs; if you change a hot path (filtering, virtualisation, playback IPC), describe what you exercised in the PR.

## Pull requests

- One concern per PR. Easier to review, easier to revert.
- Reference the issue you're fixing in the description.
- Keep commit messages short and informal (lower-case is fine). The PR description is where the *why* lives.
- Run the checks above, mention any platforms you tested.

## Mobile

If you're touching the iOS or Android bridge, expect device-specific gotchas — audio session ordering on iOS (`mpv_init` must precede `init_audio` or playback runs ~8.8% fast at 44.1 kHz), ExoPlayer's single-thread contract on Android (every `player.*` call must run on the main looper), MediaSession ownership across the plugin / foreground service boundary. The existing comments around those code paths describe the constraints; please don't rip them out.

Mobile-bridge PRs need a smoke test on at least one real device or simulator; emulator-only is OK for code review but not for "ready to merge".

## Reporting bugs / requesting features

[Issue templates](.github/ISSUE_TEMPLATE/) cover the common shapes. Include the platform, the Plex Media Server version, and steps to reproduce. Logs from the dev console (browser inspector) often contain the actual failure.
