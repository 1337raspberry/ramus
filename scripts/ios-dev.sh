#!/usr/bin/env bash
# Launch an iOS dev build under a signing-friendly bundle identifier.
#
# The committed iOS identity (tauri.conf.json + project.yml) is `com.ramus.app`,
# the release/distribution App ID. That App ID may be registered to a different
# Apple team than the one used for local development, so a normal
# `cargo tauri ios dev` / Xcode Run fails to sign it ("cannot be registered to
# your development team"). This wrapper overrides the identifier for the dev run
# ONLY, via Tauri's `--config` merge, leaving the committed/release identity
# untouched.
#
# Override the dev identifier by exporting RAMUS_IOS_DEV_IDENTIFIER.
# The signing team itself comes from gen/apple/signing.local.env (see
# scripts/regen-ios-project.sh).
#
# Usage: ./scripts/ios-dev.sh "<device or simulator name>"   [extra cargo args…]
set -euo pipefail
cd "$(dirname "$0")/.."

DEV_IDENTIFIER="${RAMUS_IOS_DEV_IDENTIFIER:-net.blueshelter.ramus}"
echo "ios-dev: building with dev identifier ${DEV_IDENTIFIER} (release identity com.ramus.app is unchanged)"

exec cargo tauri ios dev "$@" --config "{\"identifier\":\"${DEV_IDENTIFIER}\"}"
