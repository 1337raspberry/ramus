#!/usr/bin/env python3
"""Bundle libmpv and every transitive non-glibc dependency into the Linux
AppImage so the release artifact is self-contained.

Run from CI before invoking tauri-action on the Linux runner. The script:

1. Walks `libmpv.so.2`'s transitive deps via `ldd`, following resolved
   paths.
2. Filters out two families that must come from the host:
     - glibc-family + loader (libc, libm, libpthread, libdl, libstdc++,
       libgcc_s, libresolv, librt, libutil, ld-linux*) — coupled to the
       target system's dynamic loader.
     - a narrow set of host-coupled runtime libs (libdrm, libgbm; the
       glib/gio stack; libfontconfig). See `HOST_PROVIDED_PREFIXES`.
   Everything else — including libmpv's direct DT_NEEDED audio backends
   (libjack/libpulse/libpipewire/libasound), Wayland/X11/xkbcommon, the
   Khronos GL/EGL loaders, libplacebo, libass, ffmpeg — is bundled, so
   libmpv loads on a stock desktop install regardless of which audio
   daemons or display servers happen to be installed.
3. Copies each remaining lib into `ramus-tauri/linux-libs/`, named by
   DT_SONAME (e.g. `libmpv.so.1`, not the versioned real file
   `libmpv.so.1.109.0`). The dynamic linker resolves NEEDED entries by
   SONAME and our explicit `dlopen` uses the soname too; bundling under
   the resolved real filename leaves the lib unfindable even though it
   ships in the AppImage.
4. Runs `patchelf --set-rpath '$ORIGIN'` on each copy so the bundled libs
   resolve their own NEEDED deps relative to their runtime location (the
   AppImage's mounted /usr/lib dir).
5. Writes `ramus-tauri/tauri.linux.conf.json` with
   `bundle.linux.appimage.files` mapping each lib to `/usr/lib/<soname>`
   in the AppImage. Tauri auto-merges this platform-conf at build time.

At runtime, AppImage extracts to /tmp/.mount_xxx/, and the binary lives
at /tmp/.mount_xxx/usr/bin/ramus. `MpvLib::load()` searches for libmpv at
`<exe_dir>/../lib/libmpv.so.2`, which resolves to
/tmp/.mount_xxx/usr/lib/libmpv.so.2. Once libmpv is loaded, its NEEDED
deps resolve via $ORIGIN to the same dir.

The .deb and .rpm packages are not affected by this script — those use
the system libmpv via the package manager dependencies declared in
`tauri.conf.json > bundle.linux.{deb,rpm}.depends`. Only the AppImage,
which is portable across distros, gets the bundled libs.
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

# glibc-family + loader libs — never bundled in an AppImage. They are
# tied to the target system's loader/glibc version and bundling them
# crashes across distros. AppImage convention: defer to the host.
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

# Host-provided runtime families — must NOT be bundled even though
# libmpv pulls them in transitively. Matched as soname prefixes so every
# minor-version variant is covered.
#
# The skip list is deliberately narrow. Earlier attempts to also skip
# libEGL/libGL, wayland, X11/xcb, libxkbcommon, and audio client libs
# (libjack/libpulse/libpipewire/libasound) broke loading entirely — most
# of those are direct DT_NEEDED entries on libmpv.so.{1,2}, and Ubuntu
# 22.04's libmpv build links libjack in particular which isn't on most
# desktop hosts by default (JACK is opt-in audio routing — Kubuntu 26
# ships without it). The libs we DO need to skip:
#
# - libdrm / libgbm: kernel-coupled mesa-internal ABI. Bundling older
#   versions makes the host's iris_dri.so / radeonsi_dri.so etc. fail to
#   load with `did not find extension DRI_Mesa version 1`, which then
#   cascades to `No provider of glGenTextures` (Ubuntu 26) and
#   `EGL_BAD_PARAMETER` (Fedora 42). Skipping these lets the host's
#   libdrm/libgbm match the host's DRI drivers.
#
# - glib / gio stack: gvfs and gio modules in `/usr/lib/.../gio/modules/`
#   are loaded by the host's webkit/GTK stack and link against the host
#   glib. If we ship an older libglib-2.0.so.0 with RPATH=$ORIGIN, the
#   loader caches it; the host's libgvfscommon.so then trips on a
#   missing `g_task_set_static_name` (added in glib 2.76) on Ubuntu 26+.
#
# - libfontconfig: reads `/etc/fonts/fonts.conf` and the host's
#   fc-cache; the bundled copy doesn't see the host's installed fonts.
HOST_PROVIDED_PREFIXES = (
    "libgbm.so", "libdrm.so", "libdrm_",
    "libglib-2.0.so", "libgobject-2.0.so", "libgio-2.0.so",
    "libgmodule-2.0.so", "libgthread-2.0.so",
    "libfontconfig.so",
)


def is_skippable(soname: str) -> bool:
    if soname in GLIBC_FAMILY:
        return True
    if LD_LINUX_RE.match(soname):
        return True
    return any(soname.startswith(p) for p in HOST_PROVIDED_PREFIXES)


def get_soname(path: Path) -> str:
    """Return the DT_SONAME of `path`, or its filename if it has none.

    The dynamic linker resolves DT_NEEDED entries by SONAME, not by the
    versioned real filename. e.g. libmpv records `NEEDED libavcodec.so.58`,
    not `libavcodec.so.58.134.100`. Bundling under the real name leaves
    every transitive load unresolvable — and `dlopen("libmpv.so.1")` fails
    too because the symlink to the real file isn't present in the AppImage.
    Falling back to the filename keeps libs without a SONAME (rare, but
    possible) working.
    """
    out = subprocess.check_output(
        ["patchelf", "--print-soname", str(path)], text=True
    ).strip()
    return out or path.name


def ldd_deps(path: Path) -> list[Path]:
    """Return the resolved paths of every shared lib `path` links against.

    `ldd` output:
        libfoo.so.2 => /lib/x86_64-linux-gnu/libfoo.so.2 (0x00007f...)
        linux-vdso.so.1 (0x00007ff...)
        /lib64/ld-linux-x86-64.so.2 (0x00007f...)

    Grabs `<soname> => <path>` lines, filters out glibc-family sonames,
    and skips lines without a resolved path (vdso, ld-linux loader
    entries that lack a `=>`, or `not found` markers).
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

    Prefers `libmpv.so.2` (Ubuntu 24.04+, Fedora 38+, Arch) and falls
    back to `libmpv.so.1` (Ubuntu 22.04 LTS, older distros). The soname
    symlink points at the actual versioned file.
    """
    search_dirs = [
        Path("/usr/lib/x86_64-linux-gnu"),  # Debian / Ubuntu multiarch (x86_64)
        Path("/usr/lib/aarch64-linux-gnu"),  # Debian / Ubuntu multiarch (arm64)
        Path("/usr/lib64"),  # Fedora / RHEL
        Path("/usr/lib"),  # Arch and others
    ]
    # Newer first — libmpv2 wins when both are installed.
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
    # Track already-copied SONAMEs. walk_transitive dedupes by resolved
    # real path, so symlinked sonames collapse to one file — but two
    # different real files can still share a SONAME (e.g. a multi-arch
    # install with `/usr/lib/x86_64-linux-gnu/libfoo.so.2` and
    # `/usr/lib/libfoo.so.2`). Fail loudly rather than silently overwrite
    # and ship whichever one happened to be last in BFS order.
    used_sonames: dict[str, Path] = {}
    for src in libs:
        soname = get_soname(src)
        dst = WORKDIR / soname
        if soname in used_sonames:
            print(
                f"ERROR: SONAME collision on {soname}: already bundled "
                f"{used_sonames[soname]}, now trying to bundle {src}. "
                f"walk_transitive should not have returned both of these — "
                f"investigate the dependency graph.",
                file=sys.stderr,
            )
            return 1
        used_sonames[soname] = src
        shutil.copy2(src, dst)
        dst.chmod(0o644)
        # Set RPATH to $ORIGIN so the dynamic linker resolves this lib's
        # own NEEDED deps relative to its own location, not via the host's
        # ld.so.cache. Required for cross-distro portability.
        subprocess.run(
            ["patchelf", "--set-rpath", "$ORIGIN", str(dst)], check=True
        )
        # AppImage destination: each lib lands at /usr/lib/<soname>. The
        # dynamic linker (and our explicit `dlopen("libmpv.so.1")`) looks
        # libs up by SONAME, not by the versioned real filename — bundling
        # as `libmpv.so.1.109.0` instead of `libmpv.so.1` would leave the
        # file unfindable even though it ships in the AppImage. Tauri's
        # `bundle.linux.appimage.files` map keys are absolute paths inside
        # the AppImage; values are paths relative to tauri.conf.json.
        files_config[f"/usr/lib/{soname}"] = f"linux-libs/{soname}"

    config = {
        "$schema": "https://schema.tauri.app/config/2",
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
