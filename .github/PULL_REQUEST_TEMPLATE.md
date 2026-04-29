<!-- Thanks for the PR! Please fill in the sections below — feel free to delete any that don't apply. -->

## What this changes

<!-- One or two sentences on the user-visible behaviour change, or the internal cleanup. -->

## Why

<!-- Link the issue if there is one. If not, give the motivation. The why ages better than the what. -->

## Testing

<!-- How did you verify this? Platforms, scenarios, edge cases. "Manually smoke-tested on macOS" is fine for small UI tweaks; cross-platform changes need more. -->

- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test -p ramus-core`
- [ ] `cd ui && npx tsc --noEmit`
- [ ] If dependencies changed: regenerated `THIRD_PARTY_LICENSES.md` (`python3 scripts/generate-third-party-licenses.py`)

## Platforms exercised

<!-- Tick all you actually ran the change on. -->

- [ ] macOS
- [ ] Windows
- [ ] Linux
- [ ] iOS (simulator / device)
- [ ] Android (emulator / device)

## Checklist

- [ ] Commit messages are concise and informal — describe the *why*, not the diff.
- [ ] No commented-out code, no leftover debug prints, no inline `// fix me later` comments.
- [ ] No new abstractions without three concrete callers.
- [ ] Any new dependency is justified in the description.
