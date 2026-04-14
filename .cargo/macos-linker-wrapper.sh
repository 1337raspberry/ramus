#!/usr/bin/env bash
# Cargo linker wrapper referenced from .cargo/config.toml on macOS targets.
#
# Runs rustc's real linker (cc, which on macOS resolves to clang), then
# re-signs the output with `codesign --force --sign -` to strip the
# `linker-signed` flag ld automatically adds to ad-hoc signatures.
#
# On macOS 26, ad-hoc signed binaries carrying the linker-signed flag
# (flags 0x20002 instead of plain 0x2) running from untrusted locations —
# anything outside /Applications/, including target/debug/ and
# target/release/ — get killed by the in-kernel code signing monitor the
# moment they try to `dlopen` an ad-hoc dylib from a brew-style path like
# /opt/homebrew/. The error is `CODESIGNING / Invalid Page` on the first
# page read of the mapped library. Re-signing with `codesign --force
# --sign -` replaces the signature with a plain ad-hoc one the kernel
# accepts.
#
# The same binary running from /Applications/ loads fine. See CLAUDE.md
# "macOS code signing gotcha" for the full diagnosis.
#
# Codesign errors are swallowed because some rustc link invocations produce
# non-Mach-O outputs that codesign refuses to touch. The real linker has
# already succeeded by this point, so this step cannot mask a compile
# error.

set -e

"${CC:-cc}" "$@"

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
