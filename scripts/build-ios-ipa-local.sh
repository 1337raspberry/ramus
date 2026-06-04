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

# `--features tauri/custom-protocol` is REQUIRED for a production build.
# Without it, the `tauri` crate's build.rs flips on `cfg(dev)`, tauri-codegen
# embeds zero frontend assets, and the binary is hard-coded to fetch
# `devUrl` (http://localhost:5173) from `tauri.conf.json` at runtime — so
# the sideloaded app boots into a "couldn't reach dev server" error screen.
# `cargo tauri ios build` adds this feature for you (see
# tauri-cli/src/interface/rust.rs::build_options); we bypass that CLI here
# to dodge the xcode-script RPC handshake, so we have to add it ourselves.
echo ">>> [2/6] cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib --features tauri/custom-protocol"
cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib --features tauri/custom-protocol

# The Xcode build configurations are named lowercase `release`/`debug`
# (see project.yml `configs:`), so LIBRARY_SEARCH_PATHS resolves
# Externals/arm64/release. Place the release-built staticlib there; the
# `debug` copy is cheap insurance against a stray debug-config link
# step. The Rust staticlib is identical either way.
for CONFIG in release debug; do
    DEST_DIR="ramus-tauri/gen/apple/Externals/arm64/$CONFIG"
    mkdir -p "$DEST_DIR"
    cp "target/aarch64-apple-ios/release/libramus_tauri.a" "$DEST_DIR/libapp.a"
done
echo "     placed libapp.a in Externals/arm64/{release,debug}/"

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
    -configuration release \
    -destination 'generic/platform=iOS' \
    -derivedDataPath "$DERIVED_DATA" \
    CODE_SIGNING_ALLOWED=NO \
    CODE_SIGNING_REQUIRED=NO \
    CODE_SIGN_IDENTITY="" \
    PROVISIONING_PROFILE_SPECIFIER="" \
    DEVELOPMENT_TEAM="" \
    ENABLE_DEBUG_DYLIB=NO

# Find the .app. The build config is named lowercase `release` (see
# project.yml `configs:`), so the products land in `release-iphoneos/`.
# Search the capitalised spellings and the debug variants too as a
# safety net — if the config name or the -configuration flag ever drift
# out of sync again, pick up whatever Xcode produced and let the check
# below flag a non-release wrapper.
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
    echo "::warning:: Xcode produced $ACTUAL_CFG instead of release — IPA installs fine but the Swift wrapper is debug-config (larger, unoptimised); the bundled libapp.a is release-built regardless. Check that xcodebuild's -configuration matches the config name in project.yml." >&2
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
