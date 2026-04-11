#!/usr/bin/env bash
# Cargo linker wrapper — referenced from .cargo/config.toml on macOS targets.
#
# Runs rustc's real linker (cc, which on macOS resolves to clang), then
# re-signs the output with a plain `codesign --force --sign -` to strip the
# `linker-signed` flag that ld automatically adds to ad-hoc signatures.
#
# Why this exists: on macOS 26, ad-hoc signed binaries carrying the
# linker-signed flag (flags 0x20002 instead of plain 0x2) running from
# untrusted locations — which is anything outside /Applications/, including
# target/debug/ and target/release/ — get killed by the in-kernel code
# signing monitor the moment they try to `dlopen` an ad-hoc dylib from a
# brew-style path like /opt/homebrew/. The error is `CODESIGNING / Invalid
# Page` on the first page read of the mapped library. Re-signing with
# `codesign --force --sign -` replaces the signature in place with a plain
# ad-hoc one, which the kernel accepts.
#
# The same binary running from /Applications/ would load fine, which is why
# the installed 0.8.0 release doesn't trip this (once the bundler fix for
# the install_name_tool corruption lands), but `cargo tauri dev` does. See
# CLAUDE.md "macOS code signing gotcha" for the full history.
#
# We swallow codesign errors because some rustc link invocations produce
# non-Mach-O outputs that codesign refuses to touch — those are harmless to
# skip. The real linker has already succeeded by this point, so nothing we
# do here can mask a real compile error.

set -e

# cc is rustc's default linker on macOS (resolves to clang via xcrun).
"${CC:-cc}" "$@"

# Extract the output path from the linker args (`-o <path>`).
prev=""
out=""
for arg in "$@"; do
    if [[ "$prev" == "-o" ]]; then
        out="$arg"
        break
    fi
    prev="$arg"
done

if [[ -n "$out" && -f "$out" ]]; then
    codesign --force --sign - "$out" 2>/dev/null || true
fi
