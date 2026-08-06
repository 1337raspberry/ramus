#!/usr/bin/env bash
# Regenerate the iOS Xcode project from project.yml, with the workspace
# version from Cargo.toml exported as RAMUS_VERSION so xcodegen can
# substitute ${RAMUS_VERSION} into Info.plist (CFBundleShortVersionString
# / CFBundleVersion). Run after a clean clone or any time the workspace
# version is bumped.
#
# Set RAMUS_FLAVOR=dev to generate the side-by-side dev project instead
# of the shipping one (see scripts/ios-flavor.sh). Unset means stable,
# so opening Xcode after a plain regen always gives the real app.
set -euo pipefail

cd "$(dirname "$0")/.."

# shellcheck source=ios-flavor.sh
. scripts/ios-flavor.sh "${RAMUS_FLAVOR:-stable}"

RAMUS_VERSION="$(awk '/^\[workspace\.package\]/{f=1; next} f && /^version[[:space:]]*=/{gsub(/[",[:space:]]/, "", $3); print $3; exit}' Cargo.toml)"

if [[ -z "${RAMUS_VERSION}" ]]; then
  echo "regen-ios-project: failed to read version from Cargo.toml" >&2
  exit 1
fi

export RAMUS_VERSION
echo "regen-ios-project: RAMUS_VERSION=${RAMUS_VERSION} flavour=${RAMUS_FLAVOR}"
echo "regen-ios-project: ${RAMUS_PRODUCT_NAME}.app / ${RAMUS_BUNDLE_ID} / ${RAMUS_APPICON}"

cd ramus-tauri/gen/apple

# xcodegen validates that every `sources:` path in project.yml exists on
# disk before generating, otherwise it fails with "missing source
# directory". Two such paths are absent on a fresh clone (and on CI):
#
#   Externals/  — fully gitignored; populated later in the build by
#                 xcodebuild's preBuildScript, which runs `cargo tauri
#                 ios xcode-script` and drops libapp.a in here per-arch.
#   assets/     — git doesn't track empty dirs; populated by `cargo
#                 tauri build` which copies licenses into assets/_up_/.
#
# Pre-create them empty so xcodegen passes spec validation; the real
# contents land later in the build.
mkdir -p Externals assets

xcodegen generate --spec project.yml

# xcodegen leaves an unresolved ${VAR} in the output when the variable is
# unset, and Xcode then expands it as an empty build setting instead of
# failing — which yields an app with a blank bundle identifier that dies
# at install time with nothing pointing at the cause. Assert the three
# flavour settings actually made it through.
PBXPROJ="ramus-tauri.xcodeproj/project.pbxproj"
check_setting() {
    if ! grep -q "^[[:space:]]*$1 = $2;" "$PBXPROJ"; then
        echo "regen-ios-project: $1 is not '$2' in the generated project." >&2
        echo "  The flavour variables did not substitute — check scripts/ios-flavor.sh." >&2
        exit 1
    fi
}
check_setting PRODUCT_NAME "${RAMUS_PRODUCT_NAME}"
check_setting PRODUCT_BUNDLE_IDENTIFIER "${RAMUS_BUNDLE_ID}"
check_setting ASSETCATALOG_COMPILER_APPICON_NAME "${RAMUS_APPICON}"
