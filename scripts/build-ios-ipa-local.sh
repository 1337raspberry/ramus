#!/usr/bin/env bash
# Build ad-hoc-signed iOS .ipa files locally, mirroring exactly what the
# release workflow does in CI. Use this to iterate on iOS build issues
# without burning macOS GitHub Actions minutes (10x weighted, and the
# free quota is meagre).
#
# Builds BOTH flavours by default (see scripts/ios-flavor.sh):
#
#   ramus_<version>_ios-adhoc.ipa       com.ramus.app      "ramus"
#   ramus_<version>_ios-dev-adhoc.ipa   com.ramus.app.dev  "ramusDEV"
#
# They install side by side on one device with separate icons, separate
# data containers and separate keychain scopes, so a dev build can be
# tested without disturbing a stable install's cached library. Pass
# --stable-only or --dev-only to build just one.
#
# The frontend bundle and the Rust staticlib are flavour-independent, so
# they are built once and shared; only the Xcode wrapper, signing and
# packaging run per flavour.
#
# Output: ramus-tauri/gen/apple/build/*.ipa
#
# This script temporarily patches ramus-tauri/gen/apple/project.yml (to
# strip the `cargo tauri ios xcode-script` preBuildScript that needs a
# parent CLI process) and restores it from git on exit, then regenerates
# the Xcode project on the stable flavour — your normal
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
APPLE_DIR="$ROOT/ramus-tauri/gen/apple"

FLAVORS=(stable dev)
case "${1:-}" in
    --stable-only) FLAVORS=(stable) ;;
    --dev-only) FLAVORS=(dev) ;;
    "") ;;
    *)
        echo "usage: $0 [--stable-only|--dev-only]" >&2
        exit 1
        ;;
esac

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
# exit time (later steps `cd` elsewhere, and a relative path checkout
# from there would silently miss the file). We also deliberately do NOT
# swallow checkout errors — a trap that quietly fails to restore is
# worse than a trap that announces it.
#
# The regen afterwards is what leaves the generated .xcodeproj on the
# stable flavour. That project is gitignored, so nothing else would put
# it back, and one left on the dev flavour would silently hand Xcode and
# `cargo tauri ios dev` a dev-identified build for the rest of the day.
restore_project_state() {
    echo ">>> Restoring project.yml from git"
    if ! git -C "$ROOT" checkout ramus-tauri/gen/apple/project.yml; then
        echo "!!! Failed to restore project.yml — run \`git checkout ramus-tauri/gen/apple/project.yml\` manually before retrying" >&2
        return
    fi
    echo ">>> Restoring the Xcode project to the stable flavour"
    if ! (cd "$ROOT" && RAMUS_FLAVOR=stable ./scripts/regen-ios-project.sh >/dev/null); then
        echo "!!! Failed to regenerate the stable Xcode project — run \`./scripts/regen-ios-project.sh\` manually before using Xcode or \`cargo tauri ios dev\`" >&2
    fi
}
trap restore_project_state EXIT

echo ">>> [1/4] Building frontend (pnpm run build)"
(cd ui && pnpm run build)

# `--features tauri/custom-protocol` is REQUIRED for a production build.
# Without it, the `tauri` crate's build.rs flips on `cfg(dev)`, tauri-codegen
# embeds zero frontend assets, and the binary is hard-coded to fetch
# `devUrl` (http://localhost:5173) from `tauri.conf.json` at runtime — so
# the sideloaded app boots into a "couldn't reach dev server" error screen.
# `cargo tauri ios build` adds this feature for you (see
# tauri-cli/src/interface/rust.rs::build_options); we bypass that CLI here
# to dodge the xcode-script RPC handshake, so we have to add it ourselves.
echo ">>> [2/4] cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib --features tauri/custom-protocol"
cargo build --target aarch64-apple-ios --release -p ramus-tauri --lib --features tauri/custom-protocol

# The Xcode build configurations are named lowercase `release`/`debug`
# (see project.yml `configs:`), so LIBRARY_SEARCH_PATHS resolves
# Externals/arm64/release. Place the release-built staticlib there; the
# `debug` copy is cheap insurance against a stray debug-config link
# step. The Rust staticlib is identical either way — and identical
# across flavours, which is why this is built once for both.
for CONFIG in release debug; do
    DEST_DIR="ramus-tauri/gen/apple/Externals/arm64/$CONFIG"
    mkdir -p "$DEST_DIR"
    cp "target/aarch64-apple-ios/release/libramus_tauri.a" "$DEST_DIR/libapp.a"
done
echo "     placed libapp.a in Externals/arm64/{release,debug}/"

echo ">>> [3/4] Patching project.yml (drop tauri xcode-script preBuildScript)"
ruby -ryaml -e "
  path = 'ramus-tauri/gen/apple/project.yml'
  d = YAML.load_file(path)
  d['targets']['ramus-tauri_iOS'].delete('preBuildScripts')
  File.write(path, d.to_yaml)
"

echo ">>> [4/4] Building ${#FLAVORS[@]} flavour(s): ${FLAVORS[*]}"

BUILT_IPAS=()

for FLAVOR in "${FLAVORS[@]}"; do
    # shellcheck source=ios-flavor.sh
    . "$ROOT/scripts/ios-flavor.sh" "$FLAVOR"

    echo
    echo ">>> [$FLAVOR] Regenerating Xcode project"
    ./scripts/regen-ios-project.sh

    echo ">>> [$FLAVOR] xcodebuild build (no signing, debug-dylib off)"
    # DerivedData is shared across flavours on purpose: because each
    # flavour has its own PRODUCT_NAME, their products land at different
    # paths and neither invalidates the other, so both stay warm without
    # paying for a second 2 GB derived-data tree.
    DERIVED_DATA="$APPLE_DIR/build/DerivedData"
    xcodebuild build \
        -project "$APPLE_DIR/ramus-tauri.xcodeproj" \
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
        CANDIDATE="$DERIVED_DATA/Build/Products/$CFG_DIR/${RAMUS_PRODUCT_NAME}.app"
        if [ -d "$CANDIDATE" ]; then
            UNSIGNED_APP="$CANDIDATE"
            ACTUAL_CFG="$CFG_DIR"
            break
        fi
    done

    if [ -z "$UNSIGNED_APP" ]; then
        echo "build-ios-ipa-local: no ${RAMUS_PRODUCT_NAME}.app under $DERIVED_DATA/Build/Products" >&2
        find "$DERIVED_DATA/Build/Products" -maxdepth 3 -name "*.app" 2>/dev/null || true
        exit 1
    fi

    if [[ "$ACTUAL_CFG" != Release-* && "$ACTUAL_CFG" != release-* ]]; then
        echo "::warning:: Xcode produced $ACTUAL_CFG instead of release — IPA installs fine but the Swift wrapper is debug-config (larger, unoptimised); the bundled libapp.a is release-built regardless. Check that xcodebuild's -configuration matches the config name in project.yml." >&2
    fi
    echo "     using app from $ACTUAL_CFG/"

    echo ">>> [$FLAVOR] Packaging IPA + ad-hoc signing"
    # An IPA is a zip whose payload must sit under a directory literally
    # named `Payload`, so each flavour gets its own staging root holding
    # one.
    STAGE="$APPLE_DIR/build/stage-$FLAVOR"
    rm -rf "$STAGE"
    mkdir -p "$STAGE/Payload"
    cp -R "$UNSIGNED_APP" "$STAGE/Payload/"

    # Sign embedded frameworks deepest-first, then the .app last so its
    # signature seals over the freshly signed dependencies.
    find "$STAGE/Payload/${RAMUS_PRODUCT_NAME}.app/Frameworks" -name "*.framework" -depth \
        -exec codesign --force --sign - --timestamp=none {} \;
    codesign --force --sign - --timestamp=none "$STAGE/Payload/${RAMUS_PRODUCT_NAME}.app"

    if [[ "$FLAVOR" == "dev" ]]; then
        IPA="ramus_${RAMUS_VERSION}_ios-dev-adhoc.ipa"
    else
        IPA="ramus_${RAMUS_VERSION}_ios-adhoc.ipa"
    fi

    rm -f "$APPLE_DIR/build/$IPA"
    (cd "$STAGE" && zip -rqy "$APPLE_DIR/build/$IPA" Payload)
    rm -rf "$STAGE"

    SIZE=$(stat -f%z "$APPLE_DIR/build/$IPA")
    SIZE_MB=$(echo "scale=1; $SIZE/1024/1024" | bc)
    echo "     ${IPA} (${SIZE_MB} MB)"
    BUILT_IPAS+=("$IPA")
done

echo
echo "Done. Built ${#BUILT_IPAS[@]} IPA(s) in $APPLE_DIR/build:"
for IPA in "${BUILT_IPAS[@]}"; do
    echo "  $IPA"
done

# Only the stable flavour is a release artifact — a dev build carries a
# different bundle identifier and has no business on a public release.
for IPA in "${BUILT_IPAS[@]}"; do
    if [[ "$IPA" != *-dev-adhoc.ipa ]]; then
        echo
        echo "Upload to draft GitHub release with:"
        echo "  gh release upload v${RAMUS_VERSION} ramus-tauri/gen/apple/build/${IPA} --clobber"
    fi
done
