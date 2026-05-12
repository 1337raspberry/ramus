#!/usr/bin/env bash
# Build an ad-hoc-signed iOS .ipa locally, mirroring exactly what the
# release workflow does in CI. Use this to iterate on iOS build issues
# without burning macOS GitHub Actions minutes (10x weighted, and the
# free quota is meagre).
#
# Output: ramus-tauri/gen/apple/build/ramus_<version>_ios-adhoc.ipa
#
# This script temporarily patches ramus-tauri/gen/apple/project.yml (to
# strip the `cargo tauri ios xcode-script` preBuildScript that needs a
# parent CLI process) and restores it from git on exit — your normal
# `cargo tauri ios dev` workflow is untouched.
#
# Prereqs (one-time):
#   - Xcode 16+ installed.
#   - rustup target add aarch64-apple-ios
#   - brew install xcodegen
#   - cd ui && pnpm install --frozen-lockfile (only if package.json changed)
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

RAMUS_VERSION="$(awk '/^\[workspace\.package\]/{f=1; next} f && /^version[[:space:]]*=/{gsub(/[",[:space:]]/, "", $3); print $3; exit}' Cargo.toml)"
if [[ -z "$RAMUS_VERSION" ]]; then
    echo "build-ios-ipa-local: failed to parse RAMUS_VERSION from Cargo.toml" >&2
    exit 1
fi

# Refuse to run if the user has uncommitted edits to project.yml — the
# trap below restores via `git checkout`, which would discard those.
if ! git diff --quiet ramus-tauri/gen/apple/project.yml; then
    echo "build-ios-ipa-local: ramus-tauri/gen/apple/project.yml has uncommitted changes." >&2
    echo "  Stash or commit them first (the script git-checkouts the file at the end)." >&2
    exit 1
fi

# Restore project.yml on exit — success, failure, or interrupt. Without
# this, `cargo tauri ios dev` would fail next time you run it because
# the patched project.yml is missing the preBuildScript it depends on.
#
# Uses absolute path via $ROOT so the trap works regardless of cwd at
# exit time (later steps `cd` into ramus-tauri/gen/apple/build/, and a
# relative path checkout from there would silently miss the file). We
# also deliberately do NOT swallow checkout errors — a trap that
# quietly fails to restore is worse than a trap that announces it.
restore_project_yml() {
    echo ">>> Restoring project.yml from git"
    if ! git -C "$ROOT" checkout ramus-tauri/gen/apple/project.yml; then
        echo "!!! Failed to restore project.yml — run \`git checkout ramus-tauri/gen/apple/project.yml\` manually before retrying" >&2
    fi
}
trap restore_project_yml EXIT

echo ">>> [1/6] Building frontend (pnpm run build)"
(cd ui && pnpm run build)

echo ">>> [2/6] cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib"
cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib

# Place libapp.a in BOTH Release and Debug slots. Xcode 16 sometimes
# links Debug variants as part of a Release build (debug-symbols
# bundle, mergeable-libs pre-pass, scheme implicit deps — exact
# trigger varies by Xcode point release). The Rust staticlib is the
# same in either slot; the cost is one extra cp.
for CONFIG in Release Debug; do
    DEST_DIR="ramus-tauri/gen/apple/Externals/arm64/$CONFIG"
    mkdir -p "$DEST_DIR"
    cp "target/aarch64-apple-ios/release/libramus_tauri.a" "$DEST_DIR/libapp.a"
done
echo "     placed libapp.a in Externals/arm64/{Release,Debug}/"

echo ">>> [3/6] Patching project.yml (drop tauri xcode-script preBuildScript)"
ruby -ryaml -e "
  path = 'ramus-tauri/gen/apple/project.yml'
  d = YAML.load_file(path)
  d['targets']['ramus-tauri_iOS'].delete('preBuildScripts')
  File.write(path, d.to_yaml)
"

echo ">>> [4/6] Regenerating Xcode project"
./scripts/regen-ios-project.sh

echo ">>> [5/6] xcodebuild build (no signing, debug-dylib off)"
cd ramus-tauri/gen/apple
DERIVED_DATA="$PWD/build/DerivedData"
xcodebuild build \
    -project ramus-tauri.xcodeproj \
    -scheme ramus-tauri_iOS \
    -configuration Release \
    -destination 'generic/platform=iOS' \
    -derivedDataPath "$DERIVED_DATA" \
    CODE_SIGNING_ALLOWED=NO \
    CODE_SIGNING_REQUIRED=NO \
    CODE_SIGN_IDENTITY="" \
    PROVISIONING_PROFILE_SPECIFIER="" \
    DEVELOPMENT_TEAM="" \
    ENABLE_DEBUG_DYLIB=NO

# Find the .app — Xcode 16 + xcodegen scheme combo doesn't always
# honour `-configuration Release` and may produce `debug-iphoneos/`
# instead. The cargo-built libapp.a is the release-optimised version
# either way, so a "debug-built" wrapper still gives a usable IPA —
# just with a slightly larger main binary. Search both spellings
# (Apple's capitalisation has drifted between Xcode versions) and
# warn if we picked up a non-Release variant.
UNSIGNED_APP=""
ACTUAL_CFG=""
for CFG_DIR in "Release-iphoneos" "release-iphoneos" "Debug-iphoneos" "debug-iphoneos"; do
    CANDIDATE="$DERIVED_DATA/Build/Products/$CFG_DIR/ramus.app"
    if [ -d "$CANDIDATE" ]; then
        UNSIGNED_APP="$CANDIDATE"
        ACTUAL_CFG="$CFG_DIR"
        break
    fi
done

if [ -z "$UNSIGNED_APP" ]; then
    echo "build-ios-ipa-local: no .app under $DERIVED_DATA/Build/Products" >&2
    find "$DERIVED_DATA/Build/Products" -maxdepth 3 -name "*.app" 2>/dev/null || true
    exit 1
fi

if [[ "$ACTUAL_CFG" != Release-* && "$ACTUAL_CFG" != release-* ]]; then
    echo "::warning:: Xcode produced $ACTUAL_CFG instead of Release — IPA will install fine but the main binary is debug-config (larger, unoptimised). The bundled libapp.a is still release-built. TODO: figure out why xcodebuild -configuration Release is being ignored." >&2
fi
echo "     using app from $ACTUAL_CFG/"

echo ">>> [6/6] Packaging IPA + ad-hoc signing"
rm -rf build/Payload
mkdir -p build/Payload
cp -R "$UNSIGNED_APP" build/Payload/

# Sign embedded frameworks deepest-first, then the .app last so its
# signature seals over the freshly signed dependencies.
find build/Payload/ramus.app/Frameworks -name "*.framework" -depth \
    -exec codesign --force --sign - --timestamp=none {} \;
codesign --force --sign - --timestamp=none build/Payload/ramus.app

cd build
IPA="ramus_${RAMUS_VERSION}_ios-adhoc.ipa"
zip -rqy "$IPA" Payload
SIZE=$(stat -f%z "$IPA")
SIZE_MB=$(echo "scale=1; $SIZE/1024/1024" | bc)

echo
echo "Done. ${IPA} (${SIZE_MB} MB) at:"
echo "  $ROOT/ramus-tauri/gen/apple/build/$IPA"
echo
echo "Upload to draft GitHub release with:"
echo "  gh release upload v${RAMUS_VERSION} ramus-tauri/gen/apple/build/${IPA} --clobber"
