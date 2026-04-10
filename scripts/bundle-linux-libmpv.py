#!/usr/bin/env python3
"""
Bundle libmpv + every transitive non-glibc dependency for the Linux AppImage
so the release artifact is fully self-contained — users no longer need to
`apt install libmpv2` after grabbing the .AppImage.

Run this from CI BEFORE invoking tauri-action on the Linux runner. The script:

1. Walks `libmpv.so.2`'s transitive deps via `ldd`, following resolved paths.
2. Filters out glibc-family libs (libc, libm, libpthread, libdl, libstdc++,
   libgcc_s, libresolv, librt, libutil, ld-linux*). Bundling these in an
   AppImage causes incompatibility across distros — they're tightly coupled
   to the target system's dynamic loader and must always come from the host.
3. Copies each remaining lib into `ramus-tauri/linux-libs/`.
4. Runs `patchelf --set-rpath '$ORIGIN'` on each copy so the bundled libs
   resolve their own NEEDED deps relative to wherever they end up at runtime
   (the AppImage's mounted /usr/lib dir).
5. Writes `ramus-tauri/tauri.linux.conf.json` with `bundle.linux.appimage.files`
   mapping each lib to `/usr/lib/<basename>` in the AppImage. Tauri auto-merges
   this platform-conf at build time.

At runtime, AppImage extracts to /tmp/.mount_xxx/, and the binary lives at
/tmp/.mount_xxx/usr/bin/ramus. `MpvLib::load()` searches for libmpv at
`<exe_dir>/../lib/libmpv.so.2` (added in the same commit as this script),
which resolves to /tmp/.mount_xxx/usr/lib/libmpv.so.2 — exactly where we
placed it. Once libmpv is loaded, its NEEDED deps resolve via $ORIGIN to
the same dir.

The .deb and .rpm packages are NOT affected by this script — those use the
system libmpv via the package manager dependencies declared in
`tauri.conf.json > bundle.linux.{deb,rpm}.depends`. Only the AppImage,
which is supposed to be portable across distros, gets the bundled libs.
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
TAURI_DIR = PROJECT_ROOT / "ramus-tauri"
WORKDIR = TAURI_DIR / "linux-libs"
CONFIG_OUT = TAURI_DIR / "tauri.linux.conf.json"

# glibc-family + loader libs — never bundle these in an AppImage. They're
# tied to the target system's loader/glibc version and bundling them causes
# crashes across distros. AppImage convention: always defer to the host.
GLIBC_FAMILY = {
    "libc.so.6",
    "libm.so.6",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
    "libresolv.so.2",
    "libutil.so.1",
    "libgcc_s.so.1",
    "libstdc++.so.6",
    "libnsl.so.1",
    "libcrypt.so.1",
}
LD_LINUX_RE = re.compile(r"^ld-linux.*\.so")


def is_skippable(soname: str) -> bool:
    return soname in GLIBC_FAMILY or bool(LD_LINUX_RE.match(soname))


def ldd_deps(path: Path) -> list[Path]:
    """Return the resolved paths of every shared lib `path` links against.

    `ldd` output looks roughly like:
        libfoo.so.2 => /lib/x86_64-linux-gnu/libfoo.so.2 (0x00007f...)
        linux-vdso.so.1 (0x00007ff...)
        /lib64/ld-linux-x86-64.so.2 (0x00007f...)

    We grab `<soname> => <path>` lines, filter out glibc-family sonames,
    and skip lines without a resolved path (vdso, ld-linux loader entries
    that lack a `=>`, or `not found` markers).
    """
    out = subprocess.check_output(["ldd", str(path)], text=True)
    deps: list[Path] = []
    for line in out.splitlines():
        line = line.strip()
        if "=>" not in line:
            continue
        soname, _, rest = line.partition(" => ")
        soname = soname.strip()
        target = rest.split(" (", 1)[0].strip()
        if not target or target == "not found":
            continue
        if is_skippable(soname):
            continue
        deps.append(Path(target))
    return deps


def walk_transitive(root: Path) -> list[Path]:
    """BFS through libmpv's deps. Returns deduped resolved real paths."""
    seen: set[Path] = set()
    bundled: list[Path] = []
    queue: list[Path] = [root]
    while queue:
        current = queue.pop(0)
        real = current.resolve()
        if real in seen:
            continue
        seen.add(real)
        bundled.append(real)
        for dep in ldd_deps(real):
            if dep.resolve() not in seen:
                queue.append(dep)
    return bundled


def find_libmpv() -> Path | None:
    """Find the libmpv soname symlink on the build runner.

    Prefers `libmpv.so.2` (Ubuntu 24.04+, Fedora 38+, Arch) and falls back
    to `libmpv.so.1` (Ubuntu 22.04 LTS, older distros). The .so.0 / soname
    symlink is what we want — its target is the actual versioned file.
    """
    search_dirs = [
        Path("/usr/lib/x86_64-linux-gnu"),  # Debian / Ubuntu multiarch
        Path("/usr/lib64"),  # Fedora / RHEL
        Path("/usr/lib"),  # Arch and others
    ]
    # Newer first — if both happen to be installed we pick libmpv2.
    for major in (2, 1):
        soname = f"libmpv.so.{major}"
        for d in search_dirs:
            candidate = d / soname
            if candidate.exists():
                return candidate
    return None


def main() -> int:
    libmpv = find_libmpv()
    if libmpv is None:
        print(
            "libmpv soname (libmpv.so.2 or libmpv.so.1) not found in any "
            "standard system location — did you `apt install libmpv-dev`?",
            file=sys.stderr,
        )
        return 1

    print(f"walking deps from {libmpv}")
    libs = walk_transitive(libmpv)
    print(f"found {len(libs)} non-glibc libs to bundle")

    if WORKDIR.exists():
        shutil.rmtree(WORKDIR)
    WORKDIR.mkdir(parents=True)

    files_config: dict[str, str] = {}
    for src in libs:
        dst = WORKDIR / src.name
        shutil.copy2(src, dst)
        dst.chmod(0o644)
        # Set RPATH to $ORIGIN so the dynamic linker resolves this lib's
        # own NEEDED deps relative to its own location, not via the host's
        # ld.so.cache. This is what makes the bundle portable across
        # distros.
        subprocess.run(
            ["patchelf", "--set-rpath", "$ORIGIN", str(dst)], check=True
        )
        # AppImage destination: each lib lands at /usr/lib/<basename>.
        # Tauri's `bundle.linux.appimage.files` map keys are absolute paths
        # inside the AppImage; values are paths relative to tauri.conf.json.
        files_config[f"/usr/lib/{src.name}"] = f"linux-libs/{src.name}"

    config = {
        "$schema": "https://raw.githubusercontent.com/nicegui/nicegui/main/nicegui/static/tauri-conf-schema.json",
        "bundle": {"linux": {"appimage": {"files": files_config}}},
    }
    CONFIG_OUT.write_text(json.dumps(config, indent=2) + "\n")
    print(f"wrote {CONFIG_OUT}")

    print("\nbundled (sorted by size):")
    files = sorted(WORKDIR.iterdir(), key=lambda p: p.stat().st_size, reverse=True)
    total = 0
    for f in files:
        size = f.stat().st_size
        total += size
        print(f"  {f.name:<55} {size / 1024 / 1024:>7.2f} MB")
    print(f"\ntotal bundle: {total / 1024 / 1024:.1f} MB across {len(files)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
