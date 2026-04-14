#!/usr/bin/env python3
"""Bundle libmpv and every transitive non-system dependency into the macOS
.app so the release artifact is self-contained.

Run from CI before invoking tauri-action. The script:

1. Walks `libmpv.2.dylib`'s transitive non-system deps via `otool -L`,
   following symlinks to the real files.
2. Copies each unique real file into `ramus-tauri/macos-frameworks/`.
   Framework binaries (e.g. `Python.framework/Versions/3.14/Python`) are
   renamed to `lib<name_lowercase>.dylib` because Tauri's
   `bundle.macOS.frameworks` rejects files without a `.dylib` extension.
3. Rewrites every bundled dylib's own install ID and its references to
   peers using `@loader_path/<basename>`. Once they all live next to each
   other inside the .app's `Contents/Frameworks/` dir, the dynamic linker
   resolves them via `@loader_path` without any absolute paths to
   /opt/homebrew.
4. Writes `ramus-tauri/tauri.macos.conf.json` with the resulting frameworks
   list. Tauri 2 auto-merges platform-conf.json files at build time, so
   the dylibs land in `<app>.app/Contents/Frameworks/`.

At runtime, `MpvLib::load()` (in `ramus-tauri/src/mpv_ffi.rs`) searches
`<app>/Contents/Frameworks/libmpv.2.dylib` as one of its candidate paths.

brew compiled mpv with `--enable-vapoursynth --enable-lua
--enable-javascript`, so `libmpv.2.dylib` has `libmujs`, `libluajit`, and
`libvapoursynth-script` in its `LC_LOAD_DYLIB` table — not optional
dlopens. Stripping them would make libmpv fail to load. They are bundled
but their features never trigger at runtime since ramus only plays audio.

The Python binary that vapoursynth-script links against is 5 MB and is
renamed to `libpython.dylib`. The full Python framework with stdlib
(~30 MB) is not bundled — only the binary, which satisfies load-time
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

# Install names from macOS itself are never bundled — they exist on every
# Mac and bundling them risks version conflicts with the loader.
SYSTEM_PREFIXES = ("/System/", "/usr/lib/")


def brew_prefix(formula: str) -> Path:
    return Path(
        subprocess.check_output(["brew", "--prefix", formula], text=True).strip()
    )


def otool_deps(path: Path) -> list[str]:
    """Return the install names of every dylib `path` links against.

    `otool -L` output:
        /path/to/file.dylib:
        \tinstall_name_one (compatibility version X, current version Y)
        \tinstall_name_two (compatibility version X, current version Y)

    The first line is the file path itself; the rest start with a tab and
    are the load-command entries (the file's own install ID followed by
    every LC_LOAD_DYLIB).
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

    Scans for `cmd LC_RPATH` lines and grabs the `path` entry that appears
    within the next few lines. Callers use these to resolve `@rpath/...`
    deps against their containing binary's RPATH list.
    """
    out = subprocess.check_output(["otool", "-l", str(path)], text=True)
    rpaths: list[str] = []
    lines = out.splitlines()
    i = 0
    while i < len(lines):
        if lines[i].strip().startswith("cmd LC_RPATH"):
            # The `path` line normally appears two lines later; scan a
            # small window in case the format shifts.
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
    binary's dir), and `@rpath/...` (substitutes each LC_RPATH entry of
    the containing binary until one exists). `@executable_path/...` cannot
    be resolved in a build script and returns None.

    Homebrew's mpv formula uses absolute install names for its direct
    deps, so in practice this mostly matters for transitive deps of
    ffmpeg/libplacebo etc., which do use `@rpath` on newer brew builds.
    Unresolved entries get dropped silently by `walk_transitive` and
    produce an incomplete bundle.
    """
    if install_name.startswith("@rpath/"):
        rel = install_name[len("@rpath/"):]
        for rpath in otool_rpaths(containing):
            # An RPATH entry may itself start with @loader_path (dyld's
            # equivalent of $ORIGIN); substitute against the containing
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
    p = Path(install_name)
    return p if p.exists() else None


def bundled_basename(install_name: str) -> str:
    """Pick the filename used inside macos-frameworks/ for this install name.

    Normal dylibs (e.g. /opt/homebrew/lib/libmpv.2.dylib) keep the
    original basename. Framework binaries
    (.../Foo.framework/Versions/X/Foo) are renamed to `libfoo.dylib`
    because Tauri's frameworks config rejects entries without a .dylib
    extension.
    """
    p = Path(install_name)
    for part in p.parts:
        if part.endswith(".framework"):
            framework_name = part[: -len(".framework")]
            return f"lib{framework_name.lower()}.dylib"
    return p.name


def walk_transitive(root: Path) -> dict[str, Path]:
    """BFS through libmpv's transitive deps.

    Returns a map of install-name-as-seen-in-load-commands to resolved
    real path on disk. Multiple install names may collapse to the same
    real path (e.g. a symlink soname and an `@rpath/` ref both pointing
    at the same versioned file). The `-change` rewrite later needs every
    distinct install name, so all are kept in the map — dedupe happens at
    the real-path layer in `main()`.
    """
    seen: set[str] = set()
    result: dict[str, Path] = {}
    # Queue entries: (install_name, containing_binary). `containing` is
    # the file that referenced this install name, required so `@rpath/`
    # refs can be resolved against that file's own LC_RPATH list.
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
    # libavcodec.62.x.dylib) produce only one bundled file.
    real_to_install_names: dict[Path, list[str]] = defaultdict(list)
    for install_name, real_path in refs.items():
        real_to_install_names[real_path].append(install_name)

    if WORKDIR.exists():
        shutil.rmtree(WORKDIR)
    WORKDIR.mkdir(parents=True)

    # Copy each unique real file once. The bundled basename comes from the
    # first install name seen for it; multiple files mapping to the same
    # basename produce a loud warning.
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

    # Sanity check before rewriting: every non-system dep embedded in a
    # copied file must have an entry in `install_name_to_target`,
    # otherwise the `-change` pass leaves a stale absolute path (or
    # unresolved `@rpath/`) behind and the shipped .app fails at dlopen.
    # Hard-fail here rather than ship a broken bundle.
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

    # Rewrite install names so every bundled dylib references its peers
    # via @loader_path/<basename>. After this, the bundle is self-
    # contained — no absolute paths to /opt/homebrew in any file.
    for real_path, target_basename in real_to_target.items():
        path = WORKDIR / target_basename
        # IMPORTANT: snapshot deps BEFORE touching the self-id. `otool -L`
        # reports the current LC_ID_DYLIB as the first tab-indented entry,
        # so once it's rewritten to `@loader_path/<basename>` a second
        # call returns that new string — which is not in
        # install_name_to_target (keyed on the original install names).
        # Reading first keeps the snapshot consistent with the map.
        # `-change` is a no-op on LC_ID_DYLIB anyway (that's `-id`'s
        # domain), so including the original self-id in the iteration
        # below is harmless.
        deps = otool_deps(path)
        # The dylib's own install ID — what other binaries see as its name.
        install_name_tool("-id", f"@loader_path/{target_basename}", str(path))
        # Every non-system dep is guaranteed present in the map by the
        # sanity check above, so the lookup is unconditional.
        for dep in deps:
            if dep.startswith(SYSTEM_PREFIXES):
                continue
            mapped = install_name_to_target[dep]
            install_name_tool(
                "-change", dep, f"@loader_path/{mapped}", str(path)
            )

    # Re-sign every modified dylib ad-hoc. CRITICAL on Apple Silicon: any
    # `install_name_tool` edit invalidates the file's embedded code
    # signature, and on macOS 26 dyld's code signing monitor kills the
    # process with SIGKILL (Code Signature Invalid, "Invalid Page") on the
    # first page read during dlopen. Recent `install_name_tool` attempts
    # an automatic ad-hoc re-sign, but it quietly fails when the original
    # signature blob lacks padding for the new name — exactly what 0.8.0
    # hit with the brew-built libmpv stack. Explicit `codesign --force
    # --sign -` is the standard remedy and works regardless of blob
    # layout.
    #
    # Order is irrelevant: signing is per-file and doesn't care whether a
    # dylib's `@loader_path` deps exist yet. `codesign --verify --strict`
    # runs immediately after to hard-fail the build on any remaining
    # signature breakage, preventing another silent corruption shipping.
    for target_basename in real_to_target.values():
        path = WORKDIR / target_basename
        subprocess.run(
            ["codesign", "--force", "--sign", "-", str(path)],
            check=True,
        )
        subprocess.run(
            ["codesign", "--verify", "--strict", str(path)],
            check=True,
        )
    print(f"re-signed + verified {len(real_to_target)} dylibs (ad-hoc)")

    # Generate tauri.macos.conf.json — Tauri auto-merges this when
    # building for macOS. Paths are relative to tauri.conf.json's dir.
    rel_paths = sorted({f"macos-frameworks/{b}" for b in real_to_target.values()})
    config = {
        "$schema": "https://schema.tauri.app/config/2",
        "bundle": {"macOS": {"frameworks": rel_paths}},
    }
    CONFIG_OUT.write_text(json.dumps(config, indent=2) + "\n")
    print(f"wrote {CONFIG_OUT}")

    # Size-sorted summary for eyeballing bundle contents.
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
