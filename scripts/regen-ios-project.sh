#!/usr/bin/env bash
# Regenerate the iOS Xcode project from project.yml, with the workspace
# version from Cargo.toml exported as RAMUS_VERSION so xcodegen can
# substitute ${RAMUS_VERSION} into Info.plist (CFBundleShortVersionString
# / CFBundleVersion). Run after a clean clone or any time the workspace
# version is bumped.
set -euo pipefail

cd "$(dirname "$0")/.."

RAMUS_VERSION="$(awk '/^\[workspace\.package\]/{f=1; next} f && /^version[[:space:]]*=/{gsub(/[",[:space:]]/, "", $3); print $3; exit}' Cargo.toml)"

if [[ -z "${RAMUS_VERSION}" ]]; then
  echo "regen-ios-project: failed to read version from Cargo.toml" >&2
  exit 1
fi

export RAMUS_VERSION
echo "regen-ios-project: RAMUS_VERSION=${RAMUS_VERSION}"

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

exec xcodegen generate --spec project.yml
