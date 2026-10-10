#!/usr/bin/env python3
"""Generate the iOS app icon sets from the desktop icon artwork.

    python3 scripts/gen-ios-appicons.py

Writes two asset-catalog icon sets, one per build flavour (see
scripts/ios-flavor.sh):

    ramus-tauri/icons/icon.png     -> AppIcon.appiconset     (stable)
    ramus-tauri/icons/iconDEV.png  -> AppIconDev.appiconset  (dev)

The desktop artwork carries its own rounded-square silhouette with
transparent corners, which macOS and Windows display as drawn. iOS applies
its own corner mask instead, and App Store Connect rejects an app icon with
an alpha channel (ITMS-90717), so the iOS images are made opaque: every
transparent pixel takes the colour of the outermost fully opaque pixel on
its row, which carries the artwork's vertical background gradient out to
the corners. The artwork is then composited over that fill, so its
anti-aliased edge blends into the same colour and leaves no seam.

Image sizes come from AppIcon.appiconset/Contents.json (point size times
scale for each entry). The dev set shares that manifest verbatim: the set
is selected by directory name, not by the file names inside it.

Requires Pillow. Re-run after editing either source image, then commit the
result. `cargo tauri icon` also writes AppIcon.appiconset, flattened onto a
plain colour, so regenerate with this script after running it.
"""

import json
import shutil
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "ramus-tauri" / "icons"
CATALOG = ROOT / "ramus-tauri" / "gen" / "apple" / "Assets.xcassets"
MANIFEST = CATALOG / "AppIcon.appiconset" / "Contents.json"

SETS = [
    (ICONS / "icon.png", CATALOG / "AppIcon.appiconset"),
    (ICONS / "iconDEV.png", CATALOG / "AppIconDev.appiconset"),
]


def flatten(src: Image.Image) -> Image.Image:
    """Return an opaque RGB copy of `src` with its corners filled in."""
    art = src.convert("RGBA")
    width, height = art.size
    pixels = art.load()

    # Per row, the colour of the outermost fully opaque pixel on each side.
    edges: list[tuple[tuple, tuple] | None] = []
    for y in range(height):
        opaque = [x for x in range(width) if pixels[x, y][3] == 255]
        if opaque:
            edges.append((pixels[opaque[0], y], pixels[opaque[-1], y]))
        else:
            edges.append(None)

    # Rows with no opaque pixel at all (the outermost one or two) borrow the
    # nearest row that has one.
    known = [y for y, e in enumerate(edges) if e is not None]
    if not known:
        sys.exit("gen-ios-appicons: source image has no opaque pixels")
    for y in range(height):
        if edges[y] is None:
            edges[y] = edges[min(known, key=lambda k: abs(k - y))]

    fill = Image.new("RGBA", (width, height))
    fill_pixels = fill.load()
    for y, (left, right) in enumerate(edges):
        for x in range(width):
            fill_pixels[x, y] = left if x < width // 2 else right

    return Image.alpha_composite(fill, art).convert("RGB")


def sizes() -> dict[str, int]:
    """Map each icon file name in the manifest to its pixel size."""
    entries = json.loads(MANIFEST.read_text())["images"]
    out = {}
    for entry in entries:
        name = entry.get("filename")
        if not name:
            continue
        points = float(entry["size"].split("x")[0])
        scale = float(entry["scale"].rstrip("x"))
        out[name] = round(points * scale)
    return out


def main() -> None:
    targets = sizes()
    for src_path, dest in SETS:
        if not src_path.is_file():
            sys.exit(f"gen-ios-appicons: missing source icon {src_path}")
        master = flatten(Image.open(src_path))

        dest.mkdir(parents=True, exist_ok=True)
        if dest != MANIFEST.parent:
            shutil.copyfile(MANIFEST, dest / "Contents.json")
        for stale in dest.glob("*.png"):
            if stale.name not in targets:
                stale.unlink()
        for name, px in targets.items():
            master.resize((px, px), Image.LANCZOS).save(dest / name, optimize=True)
        print(f"gen-ios-appicons: wrote {len(targets)} icons to {dest.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
