#!/usr/bin/env bash
# Build the iOS-27 "Apple Intelligence" experiment: the audio assistant-schema
# tier (`@AppIntent(schema: .audio.playAudio)` etc.) in RamusSiriIntents.swift.
#
# Those `.audio.*` schemas are iOS-27-SDK symbols, so the build MUST use the
# Xcode 27 beta toolchain.
#
# IMPORTANT: cargo-tauri (cargo-mobile2) selects the toolchain from
# `xcode-select -p`, NOT the `DEVELOPER_DIR` env var — so the beta must be the
# *active* developer directory, set once with:
#
#     sudo xcode-select -s /Applications/Xcode-beta.app/Contents/Developer
#
# (switch back later with the path to Xcode.app). This script refuses to build
# unless the active toolchain reports an iOS 27 SDK, so it can't silently compile
# the schema tier against the wrong SDK (which fails with
# "type 'AssistantSchemas.Entity' has no member 'audio'").
#
# The tier is isolated behind the RAMUS_IOS27_SCHEMAS Swift condition, which this
# script bakes into the generated project; plain `./scripts/ios-dev.sh` leaves it
# off, so normal Xcode-26 builds are unaffected. The release identity
# `com.ramus.app` is overridden to a signable dev identifier for this run only,
# exactly like ios-dev.sh.
#
#   RAMUS_XCODE_BETA         beta app path (default /Applications/Xcode-beta.app)
#   RAMUS_IOS_DEV_IDENTIFIER dev bundle id (default net.blueshelter.ramus)
#
# Usage: ./scripts/ios-dev-ai.sh "<device or simulator name>"  [extra cargo args…]
set -euo pipefail
cd "$(dirname "$0")/.."

BETA="${RAMUS_XCODE_BETA:-/Applications/Xcode-beta.app}"
BETA_DEV="$BETA/Contents/Developer"

# cargo-tauri follows the ACTIVE xcode-select toolchain, so gate on that — not on
# DEVELOPER_DIR, which cargo-mobile2 ignores for SDK selection.
ACTIVE_DEV="$(xcode-select -p 2>/dev/null || true)"
ACTIVE_SDK="$(xcrun --sdk iphoneos --show-sdk-version 2>/dev/null || echo '?')"
echo "ios-dev-ai: active toolchain ${ACTIVE_DEV:-<none>} (iOS SDK ${ACTIVE_SDK})"

case "$ACTIVE_SDK" in
  27.*) : ;;
  *)
    {
      echo
      echo "ios-dev-ai: ERROR — the active Xcode toolchain has iOS SDK ${ACTIVE_SDK}, but the"
      echo "  .audio.* assistant schemas need iOS 27. cargo-tauri follows xcode-select, not"
      echo "  DEVELOPER_DIR, so switch the active toolchain to the beta first:"
      echo
      echo "      sudo xcode-select -s ${BETA_DEV}"
      echo
      echo "  then re-run this script. To switch back to the stable toolchain afterwards:"
      echo "      sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
    } >&2
    exit 1
    ;;
esac

echo "ios-dev-ai: $(xcodebuild -version | head -1)"

# Bake the schema tier into the generated project, then build.
export RAMUS_SWIFT_CONDITIONS="RAMUS_IOS27_SCHEMAS"
./scripts/regen-ios-project.sh

DEV_IDENTIFIER="${RAMUS_IOS_DEV_IDENTIFIER:-net.blueshelter.ramus}"
echo "ios-dev-ai: building with dev identifier ${DEV_IDENTIFIER} + RAMUS_IOS27_SCHEMAS"
exec cargo tauri ios dev "$@" --config "{\"identifier\":\"${DEV_IDENTIFIER}\"}"
