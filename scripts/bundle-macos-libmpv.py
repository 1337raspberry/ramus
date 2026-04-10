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


def otool_rpaths(path: Path) -> list[str]:
    """Return every LC_RPATH entry embedded in `path`.

    `otool -l` dumps load commands like:
        Load command 12
              cmd LC_RPATH
          cmdsize 40
             path /opt/homebrew/lib (offset 12)

    We scan for `cmd LC_RPATH` lines and grab the `path` entry that appears
    within the next few lines. Callers use these to resolve `@rpath/...` deps
    against their containing binary's RPATH list.
    """
    out = subprocess.check_output(["otool", "-l", str(path)], text=True)
    rpaths: list[str] = []
    lines = out.splitlines()
    i = 0
    while i < len(lines):
        if lines[i].strip().startswith("cmd LC_RPATH"):
            # The `path` line normally appears two lines later, but we scan a
            # small window in case the format ever shifts.
            for j in range(i + 1, min(i + 6, len(lines))):
                stripped = lines[j].strip()
                if stripped.startswith("path "):
                    rpath = stripped[len("path "):].rsplit(" (offset", 1)[0].strip()
                    rpaths.append(rpath)
                    break
        i += 1
    return rpaths


def resolve_dep(install_name: str, containing: Path) -> Path | None:
    """Resolve a load-command install name to a concrete file on disk.

    Handles absolute paths, `@loader_path/...` (relative to the containing
    binary's dir), and `@rpath/...` (substitutes each LC_RPATH entry of the
    containing binary until one exists). `@executable_path/...` cannot be
    resolved in a build script and returns None.

    Homebrew's mpv formula currently uses absolute install names for its
    direct deps, so in practice this mostly matters for transitive deps of
    ffmpeg/libplacebo etc. which do use `@rpath` on newer brew builds. If
    we don't resolve those, `walk_transitive` drops them silently and the
    bundle ends up incomplete.
    """
    if install_name.startswith("@rpath/"):
        rel = install_name[len("@rpath/"):]
        for rpath in otool_rpaths(containing):
            # An RPATH entry may itself start with @loader_path (dyld's
            # equivalent of $ORIGIN); substitute it against the containing
            # binary's parent directory. @executable_path inside an RPATH
            # is unresolvable in this context.
            if rpath.startswith("@loader_path"):
                rpath = str(containing.parent) + rpath[len("@loader_path"):]
            elif rpath.startswith("@executable_path"):
                continue
            candidate = Path(rpath) / rel
            if candidate.exists():
                return candidate
        return None
    if install_name.startswith("@loader_path/"):
        candidate = containing.parent / install_name[len("@loader_path/"):]
        return candidate if candidate.exists() else None
    if install_name.startswith("@executable_path/"):
        return None
    # Plain absolute path.
    p = Path(install_name)
    return p if p.exists() else None


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
    """BFS through libmpv's transitive deps.

    Returns a map of install-name-as-seen-in-load-commands → resolved real
    path on disk. Multiple install names may collapse to the same real path
    (e.g. a symlink soname AND an `@rpath/` ref both pointing at the same
    versioned file). The `-change` rewrite later needs to find every distinct
    install name, so we keep them all in the map — dedupe happens at the
    real-path layer in `main()`.
    """
    seen: set[str] = set()
    result: dict[str, Path] = {}
    # Queue entries: (install_name, containing_binary). `containing` is the
    # file that referenced this install name — needed so `@rpath/` refs can be
    # resolved against that file's own LC_RPATH list.
    queue: list[tuple[str, Path]] = [(str(root), root.resolve())]
    while queue:
        install_name, containing = queue.pop(0)
        if install_name in seen:
            continue
        seen.add(install_name)
        if install_name.startswith(SYSTEM_PREFIXES):
            continue
        resolved = resolve_dep(install_name, containing)
        if resolved is None:
            print(
                f"warn: could not resolve {install_name} (referenced from "
                f"{containing}); skipping",
                file=sys.stderr,
            )
            continue
        real = resolved.resolve()
        result[install_name] = real
        for dep in otool_deps(real):
            queue.append((dep, real))
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

    # Sanity check BEFORE we start rewriting: every non-system dep embedded in
    # a copied file must have an entry in `install_name_to_target`, otherwise
    # the `-change` pass would leave a stale absolute path (or unresolved
    # `@rpath/`) behind and the shipped .app would fail at dlopen. Hard-fail
    # here rather than warning and shipping a broken bundle.
    missing: list[tuple[str, str]] = []
    for real_path, target_basename in real_to_target.items():
        path = WORKDIR / target_basename
        for dep in otool_deps(path):
            if dep.startswith(SYSTEM_PREFIXES):
                continue
            if dep in install_name_to_target:
                continue
            missing.append((target_basename, dep))

    if missing:
        print(
            "ERROR: bundled files reference deps that were not themselves bundled:",
            file=sys.stderr,
        )
        for binary, dep in missing:
            print(f"  {binary} → {dep}", file=sys.stderr)
        print(
            "\nThis usually means walk_transitive() failed to resolve an "
            "@rpath / @loader_path dep or otherwise missed a branch of the "
            "dependency graph. Fix the resolver and rerun.",
            file=sys.stderr,
        )
        return 1

    # Rewrite install names so every bundled dylib references its peers via
    # @loader_path/<basename>. After this, the bundle is fully self-contained
    # — no absolute paths to /opt/homebrew anywhere in any of these files.
    for real_path, target_basename in real_to_target.items():
        path = WORKDIR / target_basename
        # IMPORTANT: snapshot deps BEFORE touching the self-id. `otool -L`
        # reports the current LC_ID_DYLIB as the first tab-indented entry, so
        # once we've rewritten it to `@loader_path/<basename>` a second call
        # would return that new string — and it's not in install_name_to_target
        # (which is keyed on the *original* install names we walked). Reading
        # first keeps the snapshot consistent with the map. `-change` is a
        # no-op on LC_ID_DYLIB anyway (that's exclusively `-id`'s domain), so
        # including the original self-id in the iteration below is harmless.
        deps = otool_deps(path)
        # The dylib's own install ID — what other binaries see as its name.
        install_name_tool("-id", f"@loader_path/{target_basename}", str(path))
        # Every non-system dep is guaranteed present in the map by the
        # sanity check above, so we can look up unconditionally.
        for dep in deps:
            if dep.startswith(SYSTEM_PREFIXES):
                continue
            mapped = install_name_to_target[dep]
            install_name_tool(
                "-change", dep, f"@loader_path/{mapped}", str(path)
            )

    # Generate tauri.macos.conf.json — Tauri auto-merges this when building
    # for macOS. Paths are interpreted relative to tauri.conf.json's dir.
    rel_paths = sorted({f"macos-frameworks/{b}" for b in real_to_target.values()})
    config = {
        "$schema": "https://schema.tauri.app/config/2",
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
