#!/usr/bin/env python3
"""Strip host-coupled libs from the AppImage that Tauri's bundler
shipped but that must come from the host system.

Background: `cargo tauri build --bundles appimage` invokes
`tauri-bundler`, which under the hood runs `linuxdeploy` plus
`linuxdeploy-plugin-gtk`. That plugin's job is "make GTK/webkit2gtk
portable" and bundles the full GTK runtime, including the host's
wayland/X11 client stack (libwayland-*, libxkbcommon, libX11, libxcb*,
GTK wayland input-method modules). The plugin overrides the standard
AppImage excludelist for this — its policy is "bundle everything GTK
needs, even libs the excludelist says are host-provided".

That policy breaks webkit2gtk's EGL init on distros that moved past
Ubuntu 22.04's lib versions. The loader honours each bundled lib's
RPATH=$ORIGIN and caches our copies before the host webkit/GDK init
runs; webkit's `eglGetDisplay(EGL_DEFAULT_DISPLAY)` then trips on a
mismatched libwayland-client / libxkbcommon ABI and aborts with
`could not create default EGL display: EGL_BAD_PARAMETER. Aborting...`,
leaving the window invisible (process alive, no rendered surface).
Fedora 42 hits this loudly; any modern fast-moving distro is one apt
upgrade away.

`scripts/bundle-linux-libmpv.py` already excludes these from OUR
contribution, but `bundle.linux.appimage.files` is purely additive —
we can't tell Tauri to NOT bundle something. Hence this post-process:
extract the produced AppImage, delete the offending libs, repack.

Usage:
  - Local:   `python3 scripts/strip-appimage.py` after
             `cargo tauri build --bundles appimage`.
  - CI:      runs after the tauri-action step in release.yml; the
             stripped AppImage is then re-uploaded with
             `gh release upload --clobber`.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
BUNDLE_DIR = PROJECT_ROOT / "target" / "release" / "bundle" / "appimage"

# Soname prefixes for libs that must come from the host. Mirrors the
# canonical AppImage excludelist
# (https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist)
# narrowed to the libs that linuxdeploy-plugin-gtk over-bundles AND
# that empirically break webkit2gtk's EGL init on newer distros.
# Prefix-matched so every minor-version variant is covered.
EXCLUDE_PREFIXES = (
    "libwayland",     # libwayland-{client,cursor,egl,server}.so.*
    "libxkbcommon",   # libxkbcommon.so.0, libxkbcommon-x11.so.0
    "libX11",         # libX11.so.6, libX11-xcb.so.1
    "libxcb",         # libxcb.so.1 + all libxcb-<ext>.so.0 variants
    "im-wayland",     # GTK Wayland input-method module (im-wayland*.so)
)


def find_appimage() -> Path:
    candidates = [c for c in sorted(BUNDLE_DIR.glob("*.AppImage")) if c.is_file()]
    if not candidates:
        raise SystemExit(f"no .AppImage in {BUNDLE_DIR}")
    if len(candidates) > 1:
        names = ", ".join(c.name for c in candidates)
        raise SystemExit(f"multiple .AppImages in {BUNDLE_DIR}: {names}")
    return candidates[0]


def detect_arch() -> str:
    machine = os.uname().machine
    if machine in ("x86_64", "amd64"):
        return "x86_64"
    if machine in ("aarch64", "arm64"):
        return "aarch64"
    raise SystemExit(f"unsupported arch: {machine}")


def find_or_download_appimagetool(arch: str) -> Path:
    """Locate appimagetool. Prefer PATH; cache a download otherwise.

    Repacking with a different appimagetool build than Tauri used is
    safe — Type 2 AppImage is a stable format (squashfs payload + Type
    2 runtime stub). Output is byte-different but functionally
    equivalent.
    """
    on_path = shutil.which("appimagetool")
    if on_path:
        return Path(on_path)

    cache_dir = Path.home() / ".cache" / "ramus"
    cache_dir.mkdir(parents=True, exist_ok=True)
    tool = cache_dir / f"appimagetool-{arch}"
    if tool.is_file():
        return tool

    url = (
        f"https://github.com/AppImage/appimagetool/releases/download/"
        f"continuous/appimagetool-{arch}.AppImage"
    )
    print(f"downloading appimagetool from {url}")
    urllib.request.urlretrieve(url, tool)
    tool.chmod(0o755)
    return tool


def main() -> int:
    src = find_appimage()
    print(f"stripping {src.name}")

    workdir = src.parent / "_strip"
    if workdir.exists():
        shutil.rmtree(workdir)
    workdir.mkdir()

    # --appimage-extract is built into the Type 2 AppImage runtime;
    # writes squashfs-root/ in cwd and does NOT require FUSE.
    src.chmod(0o755)
    subprocess.run(
        [str(src), "--appimage-extract"],
        cwd=workdir,
        check=True,
        stdout=subprocess.DEVNULL,
    )

    appdir = workdir / "squashfs-root"
    lib_dir = appdir / "usr" / "lib"
    if not lib_dir.is_dir():
        raise SystemExit(f"unexpected AppImage layout: no {lib_dir}")

    removed: list[str] = []
    for entry in sorted(lib_dir.iterdir()):
        if not entry.is_file():
            continue
        if any(entry.name.startswith(prefix) for prefix in EXCLUDE_PREFIXES):
            entry.unlink()
            removed.append(entry.name)

    if not removed:
        print("nothing to strip — leaving AppImage untouched")
        shutil.rmtree(workdir)
        return 0

    print(f"removed {len(removed)} libs:")
    for name in removed:
        print(f"  - {name}")

    arch = detect_arch()
    tool = find_or_download_appimagetool(arch)

    tmp_out = src.with_suffix(".AppImage.tmp")
    if tmp_out.exists():
        tmp_out.unlink()

    # appimagetool itself is an AppImage. APPIMAGE_EXTRACT_AND_RUN
    # lets it run on hosts without FUSE (CI runners, containers).
    env = {**os.environ, "ARCH": arch, "APPIMAGE_EXTRACT_AND_RUN": "1"}
    subprocess.run(
        [str(tool), str(appdir), str(tmp_out)],
        env=env,
        check=True,
    )

    src.unlink()
    shutil.move(str(tmp_out), str(src))
    src.chmod(0o755)
    print(f"wrote stripped AppImage to {src}")

    shutil.rmtree(workdir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
