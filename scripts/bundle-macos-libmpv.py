#!/usr/bin/env python3
"""
Bundle libmpv + every transitive non-system dependency for the macOS .app so
the release artifact is fully self-contained — users no longer need to
`brew install mpv` after installing ramus.

Run this from CI BEFORE invoking tauri-action. The script:

1. Walks `libmpv.2.dylib`'s transitive non-system deps via `otool -L`,
   following symlinks to the real files.
2. Copies each unique real file into `ramus-tauri/macos-frameworks/`. Framework
   binaries (e.g. `Python.framework/Versions/3.14/Python`) get renamed to
   `lib<name_lowercase>.dylib` so Tauri's `bundle.macOS.frameworks` config
   accepts them — that field rejects files without a `.dylib` extension.
3. Rewrites every bundled dylib's own install ID and its references to peers
   using `@loader_path/<basename>`. Once they all live next to each other
   inside the .app's `Contents/Frameworks/` dir, the dynamic linker resolves
   them via `@loader_path` without any absolute paths to /opt/homebrew.
4. Writes `ramus-tauri/tauri.macos.conf.json` with the resulting frameworks
   list. Tauri 2 auto-merges platform-conf.json files at build time, so when
   tauri-action runs `tauri build`, the dylibs land in
   `<app>.app/Contents/Frameworks/`.

At runtime, `MpvLib::load()` (in `ramus-tauri/src/mpv_ffi.rs`) already searches
`<app>/Contents/Frameworks/libmpv.2.dylib` as one of its candidate paths, so
no Rust changes are needed.

### Why we bundle libvapoursynth-script + Python at all

brew compiled mpv with `--enable-vapoursynth --enable-lua --enable-javascript`,
so `libmpv.2.dylib` has all three optional plugins (`libmujs`, `libluajit`,
`libvapoursynth-script`) in its `LC_LOAD_DYLIB` table — they're not optional
dlopens. Stripping them would make libmpv fail to load entirely. We bundle
them but their unused features never trigger at runtime since ramus only does
audio playback.

The Python binary that vapoursynth-script links against is 5 MB; we rename it
to `libpython.dylib`. The full Python framework with stdlib (~30 MB) is NOT
bundled — only the binary itself, which is enough to satisfy the load-time
symbol resolution.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
TAURI_DIR = PROJECT_ROOT / "ramus-tauri"
WORKDIR = TAURI_DIR / "macos-frameworks"
CONFIG_OUT = TAURI_DIR / "tauri.macos.conf.json"

# install names that come from macOS itself should NOT be copied — they exist
# on every Mac and bundling them would risk version conflicts with the loader.
SYSTEM_PREFIXES = ("/System/", "/usr/lib/")


def brew_prefix(formula: str) -> Path:
    return Path(
        subprocess.check_output(["brew", "--prefix", formula], text=True).strip()
    )


def otool_deps(path: Path) -> list[str]:
    """Return the install names of every dylib `path` links against.

    `otool -L` output looks like:
        /path/to/file.dylib:
        \tinstall_name_one (compatibility version X, current version Y)
        \tinstall_name_two (compatibility version X, current version Y)
        ...

    The first line is the file path itself; the rest start with a tab and are
    the load-command entries (the file's own install ID followed by every
    LC_LOAD_DYLIB).
    """
    out = subprocess.check_output(["otool", "-L", str(path)], text=True)
    deps: list[str] = []
    for line in out.splitlines():
        if not line.startswith("\t"):
            continue
        deps.append(line.strip().split(" (", 1)[0])
    return deps


def is_skippable(install_name: str) -> bool:
    return install_name.startswith(SYSTEM_PREFIXES) or install_name.startswith("@")


def bundled_basename(install_name: str) -> str:
    """Pick the filename we'll use inside macos-frameworks/ for this install name.

    For normal dylibs (e.g. /opt/homebrew/lib/libmpv.2.dylib) we keep the
    original basename. For framework binaries (.../Foo.framework/Versions/X/Foo)
    we rename to `libfoo.dylib` so Tauri's frameworks config accepts the file
    — it rejects entries without a .dylib extension.
    """
    p = Path(install_name)
    for part in p.parts:
        if part.endswith(".framework"):
            framework_name = part[: -len(".framework")]
            return f"lib{framework_name.lower()}.dylib"
    return p.name


def walk_transitive(root: Path) -> dict[str, Path]:
    """BFS through libmpv's deps. Returns map of install_name → resolved real path."""
    seen: set[str] = set()
    result: dict[str, Path] = {}
    queue: list[tuple[str, Path]] = [(str(root), root)]
    while queue:
        install_name, current = queue.pop(0)
        if install_name in seen:
            continue
        seen.add(install_name)
        if not current.exists():
            print(f"warn: {current} not found, skipping", file=sys.stderr)
            continue
        real = current.resolve()
        result[install_name] = real
        for dep in otool_deps(real):
            if is_skippable(dep):
                continue
            queue.append((dep, Path(dep)))
    return result


def install_name_tool(*args: str) -> None:
    subprocess.run(["install_name_tool", *args], check=True)


def main() -> int:
    libmpv = brew_prefix("mpv") / "lib" / "libmpv.2.dylib"
    if not libmpv.exists():
        print(
            f"libmpv not found at {libmpv} — did you `brew install mpv`?",
            file=sys.stderr,
        )
        return 1

    print(f"walking deps from {libmpv}")
    refs = walk_transitive(libmpv)
    print(f"found {len(refs)} install names")

    # Group by resolved real path so symlinks (libavcodec.dylib ->
    # libavcodec.62.x.dylib) only result in one bundled file.
    real_to_install_names: dict[Path, list[str]] = defaultdict(list)
    for install_name, real_path in refs.items():
        real_to_install_names[real_path].append(install_name)

    if WORKDIR.exists():
        shutil.rmtree(WORKDIR)
    WORKDIR.mkdir(parents=True)

    # Copy each unique real file once. Pick the bundled basename from the
    # first install name we saw for it; if multiple files would map to the
    # same basename we warn loudly so we can debug.
    install_name_to_target: dict[str, str] = {}
    real_to_target: dict[Path, str] = {}
    used_basenames: set[str] = set()
    for real_path, install_names in real_to_install_names.items():
        target = bundled_basename(install_names[0])
        if target in used_basenames:
            print(
                f"WARN: collision on {target} from {install_names[0]} — overwriting",
                file=sys.stderr,
            )
        used_basenames.add(target)
        dst = WORKDIR / target
        shutil.copy2(real_path, dst)
        dst.chmod(0o644)
        real_to_target[real_path] = target
        for install_name in install_names:
            install_name_to_target[install_name] = target

    print(f"copied {len(real_to_target)} unique files")

    # Rewrite install names so every bundled dylib references its peers via
    # @loader_path/<basename>. After this, the bundle is fully self-contained
    # — no absolute paths to /opt/homebrew anywhere in any of these files.
    for real_path, target_basename in real_to_target.items():
        path = WORKDIR / target_basename
        # The dylib's own install ID — what other binaries see as its name.
        install_name_tool("-id", f"@loader_path/{target_basename}", str(path))
        # Each dep that points at something we bundled gets rewritten.
        for dep in otool_deps(path):
            if is_skippable(dep):
                continue
            mapped = install_name_to_target.get(dep)
            if mapped:
                install_name_tool(
                    "-change", dep, f"@loader_path/{mapped}", str(path)
                )
            else:
                print(
                    f"WARN: {target_basename} references {dep} which wasn't bundled",
                    file=sys.stderr,
                )

    # Generate tauri.macos.conf.json — Tauri auto-merges this when building
    # for macOS. Paths are interpreted relative to tauri.conf.json's dir.
    rel_paths = sorted({f"macos-frameworks/{b}" for b in real_to_target.values()})
    config = {
        "$schema": "https://raw.githubusercontent.com/nicegui/nicegui/main/nicegui/static/tauri-conf-schema.json",
        "bundle": {"macOS": {"frameworks": rel_paths}},
    }
    CONFIG_OUT.write_text(json.dumps(config, indent=2) + "\n")
    print(f"wrote {CONFIG_OUT}")

    # Print a size-sorted summary so we can eyeball the bundle for surprises
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
