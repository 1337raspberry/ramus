#!/usr/bin/env bash
# Cargo linker wrapper referenced from .cargo/config.toml on macOS targets.
#
# Runs rustc's real linker (cc, which on macOS resolves to clang), then
# re-signs the output with `codesign --force --sign -` to strip the
# `linker-signed` flag ld automatically adds to ad-hoc signatures
# (flags 0x20002 become a plain 0x2).
#
# Early macOS 26 releases killed linker-signed binaries running outside
# /Applications/ (`CODESIGNING / Invalid Page`) when they dlopen-ed an
# ad-hoc dylib such as Homebrew's libmpv. macOS 26.6.2 no longer does: a
# linker-signed release binary loads its bundled libmpv from any location.
#
# Only unstripped outputs keep this signature, which in practice means
# dev-profile builds. Cargo's release profile defaults to
# `strip = "debuginfo"`, and rustc strips after the linker returns using
# its bundled rust-objcopy, which writes a fresh linker-signed signature.
# Release bundles get their plain ad-hoc signature from the Tauri bundler
# instead (`bundle.macOS.signingIdentity` in ramus-tauri/tauri.conf.json).
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
