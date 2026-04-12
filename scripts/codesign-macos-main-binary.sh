#!/usr/bin/env bash
# Re-sign the main binary with a plain ad-hoc signature before bundling.
# This strips the linker-signed flag that rustc/ld applies, which macOS 26+
# rejects when dlopen-ing ad-hoc dylibs from untrusted locations.
set -euo pipefail

BINARY="target/release/ramus-tauri"

if [ -f "$BINARY" ]; then
  codesign --force --sign - "$BINARY"
  echo "codesigned $BINARY"
else
  echo "warning: $BINARY not found, skipping codesign" >&2
fi
