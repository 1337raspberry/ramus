#!/usr/bin/env bash
# Generate the dev-flavour iOS app icon set from ramus-tauri/icons/iconDEV.png.
#
# `cargo tauri icon` only ever writes AppIcon.appiconset, so the dev
# flavour's separate AppIconDev.appiconset (selected at build time via
# ASSETCATALOG_COMPILER_APPICON_NAME — see scripts/ios-flavor.sh) has to
# be produced here instead.
#
# Sizes and filenames are read back off the shipping AppIcon set rather
# than hard-coded, so the two sets cannot drift apart if Xcode ever
# changes which idioms it wants.
#
# Re-run after editing icons/iconDEV.png, then commit the result.
set -euo pipefail

cd "$(dirname "$0")/.."

SRC="ramus-tauri/icons/iconDEV.png"
REF="ramus-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset"
DEST="ramus-tauri/gen/apple/Assets.xcassets/AppIconDev.appiconset"

if [[ ! -f "$SRC" ]]; then
    echo "gen-dev-appicon: missing source icon $SRC" >&2
    exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"

# Both sets use identical member filenames, so the manifest is shared
# verbatim — the set is selected by directory name, not by the names of
# the images inside it.
cp "$REF/Contents.json" "$DEST/Contents.json"

COUNT=0
for f in "$REF"/*.png; do
    name="$(basename "$f")"
    px="$(sips -g pixelWidth "$f" | awk '/pixelWidth/{print $2}')"
    sips -z "$px" "$px" "$SRC" --out "$DEST/$name" >/dev/null
    COUNT=$((COUNT + 1))
done

echo "gen-dev-appicon: wrote $COUNT icons to $DEST"
