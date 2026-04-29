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
exec xcodegen generate --spec project.yml
