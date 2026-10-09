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
   Checks that no copied file targets a newer macOS than
   `bundle.macOS.minimumSystemVersion` in `tauri.conf.json` (an error in
   CI, a warning elsewhere).
3. Rewrites every bundled dylib's own install ID and its references to
   peers using `@loader_path/<basename>`. Once they all live next to each
   other inside the .app's `Contents/Frameworks/` dir, the dynamic linker
   resolves them via `@loader_path` without any absolute paths to
   /opt/homebrew.
4. Collects the licence of every bundled library into
   `ramus-tauri/macos-licenses/`: a `NATIVE_LIBRARIES.md` manifest (file,
   Homebrew formula, version, licence, source) and, per formula, the
   licence files Homebrew installed at the keg root. Homebrew builds
   FFmpeg with `--enable-gpl --enable-version3` and bundles GPL libraries
   (x264, x265, rubberband), so the shipped app is GPL-3.0-or-later as a
   whole and has to carry each library's notices and a pointer to its
   exact source.
5. Writes `ramus-tauri/tauri.macos.conf.json` with the resulting frameworks
   list and a resource entry that copies `macos-licenses/` to
   `Contents/Resources/licenses/native/`. Tauri 2 auto-merges
   platform-conf.json files at build time, so the dylibs land in
   `<app>.app/Contents/Frameworks/`.

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
import os
import re
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent
TAURI_DIR = PROJECT_ROOT / "ramus-tauri"
WORKDIR = TAURI_DIR / "macos-frameworks"
LICENSES_DIR = TAURI_DIR / "macos-licenses"
CONFIG_OUT = TAURI_DIR / "tauri.macos.conf.json"
TAURI_CONF = TAURI_DIR / "tauri.conf.json"

# Licence files Homebrew copies from a formula's source tree into the keg
# root (COPYING, LICENSE.md, COPYING.LGPLv2.1, Copyright, ...).
LICENSE_FILE_RE = re.compile(
    r"^(licen[cs]e|copying|copyright|notice|unlicense)([._-].*)?$", re.IGNORECASE
)
NOT_LICENSE_SUFFIXES = (".cfg", ".json", ".py", ".sh")
# Licence texts tauri.conf.json already ships in `licenses/`, one folder up
# from the manifest; cited when a keg carries no licence file of its own.
SHIPPED_TEXTS = {
    "LGPL-2.1": "LICENSE.LGPL-2.1",
    "LGPL-3.0": "LICENSE.LGPL-3.0",
    "GPL-3.0": "LICENSE.GPL-3.0",
    "MPL-2.0": "LICENSE.MPL-2.0",
}


def shipped_texts_for(license_expr: str) -> list[str]:
    ids = {
        re.sub(r"(-or-later|-only|\+)$", "", tok)
        for tok in re.split(r"[\s()]+", license_expr)
    }
    return [f"../{text}" for spdx, text in SHIPPED_TEXTS.items() if spdx in ids]

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


def otool_minos(path: Path) -> str | None:
    """Return the minimum macOS version `path` was built for.

    Modern binaries carry it as `minos` in `LC_BUILD_VERSION`; binaries
    targeting macOS 10.13 or older use `version` in the legacy
    `LC_VERSION_MIN_MACOSX` instead:
              cmd LC_BUILD_VERSION
          cmdsize 32
         platform 1
            minos 14.0
    """
    out = subprocess.check_output(["otool", "-l", str(path)], text=True)
    lines = out.splitlines()
    for i, line in enumerate(lines):
        cmd = line.strip()
        if cmd == "cmd LC_BUILD_VERSION":
            key = "minos "
        elif cmd == "cmd LC_VERSION_MIN_MACOSX":
            key = "version "
        else:
            continue
        for j in range(i + 1, min(i + 6, len(lines))):
            stripped = lines[j].strip()
            if stripped.startswith(key):
                return stripped[len(key):].strip()
    return None


def version_tuple(version: str) -> tuple[int, ...]:
    """`"15"` and `"15.0"` compare equal: pad to three components."""
    parts = [int(p) for p in version.split(".")]
    return tuple(parts + [0] * (3 - len(parts)))


def minimum_system_version() -> str:
    """The app's declared minimum macOS (Info.plist `LSMinimumSystemVersion`).

    Falls back to Tauri's own default when the config leaves it unset.
    """
    config = json.loads(TAURI_CONF.read_text())
    return config.get("bundle", {}).get("macOS", {}).get(
        "minimumSystemVersion", "10.13"
    )


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


def keg_of(real: Path) -> tuple[str, str, Path] | None:
    """(formula, version, keg root) for a file inside the Homebrew Cellar."""
    parts = real.parts
    if "Cellar" not in parts:
        return None
    i = parts.index("Cellar")
    if i + 2 >= len(parts):
        return None
    return parts[i + 1], parts[i + 2], Path(*parts[: i + 3])


def source_of(formula: dict) -> str:
    """The formula's stable source: a tarball URL, or a git URL plus tag."""
    stable = (formula.get("urls") or {}).get("stable") or {}
    url = stable.get("url") or formula.get("homepage") or "unknown"
    ref = stable.get("tag") or stable.get("revision")
    return f"{url} ({ref})" if ref else url


def collect_licenses(real_to_target: dict[Path, str]) -> int:
    """Write `macos-licenses/`: a manifest of every bundled library and,
    per Homebrew formula, the licence files from its keg root. Returns the
    number of bundled files whose formula could not be determined."""
    if LICENSES_DIR.exists():
        shutil.rmtree(LICENSES_DIR)
    LICENSES_DIR.mkdir(parents=True)

    kegs: dict[str, tuple[str, Path, list[str]]] = {}
    unknown: list[str] = []
    for real, target in sorted(real_to_target.items(), key=lambda kv: kv[1]):
        keg = keg_of(real)
        if keg is None:
            unknown.append(target)
            continue
        formula, version, root = keg
        kegs.setdefault(formula, (version, root, []))[2].append(target)

    info = json.loads(
        subprocess.check_output(["brew", "info", "--json=v2", *sorted(kegs)], text=True)
    )
    meta = {f["name"]: f for f in info["formulae"]}

    rows = []
    for formula in sorted(kegs):
        version, root, files = kegs[formula]
        f = meta.get(formula, {})
        copied = []
        for entry in sorted(root.iterdir()):
            if (
                entry.is_file()
                and LICENSE_FILE_RE.match(entry.name)
                and not entry.name.lower().endswith(NOT_LICENSE_SUFFIXES)
            ):
                (LICENSES_DIR / formula).mkdir(exist_ok=True)
                shutil.copy2(entry, LICENSES_DIR / formula / entry.name)
                copied.append(entry.name)
        license_expr = f.get("license") or "unknown"
        texts = [f"`{formula}/{c}`" for c in copied]
        if not texts:
            print(f"warn: no licence file in the {formula} keg", file=sys.stderr)
            texts = [f"`{t}`" for t in shipped_texts_for(license_expr)] or ["none"]
        source = source_of(f)
        # The keg's version carries Homebrew's rebuild suffix (`0.41.0_8`).
        stable = ((f.get("versions") or {}).get("stable")) or ""
        if stable and version.split("_")[0] != stable:
            print(
                f"warn: {formula} {version} is installed but the formula is at "
                f"{stable}; its source link is for {stable}",
                file=sys.stderr,
            )
            source += f" (formula now at {stable}; the bundled build is {version})"
        rows.append(
            f"| {', '.join(files)} | {formula} | {version} | "
            f"{license_expr} | {source} | {', '.join(texts)} |"
        )
    for target in unknown:
        print(f"warn: {target} is not from the Homebrew Cellar", file=sys.stderr)
        rows.append(f"| {target} | unknown | | unknown | | |")

    manifest = [
        "# Native libraries bundled in the macOS app",
        "",
        "Generated by `scripts/bundle-macos-libmpv.py` from the Homebrew packages",
        "on the machine that built this release. Each library keeps its own",
        "licence; the licence files Homebrew installed with each package are in",
        "the folder named after it beside this file. Homebrew builds FFmpeg with",
        "`--enable-gpl --enable-version3` and mpv with its `gpl` feature, so this",
        "app as a whole is distributed under the GNU GPL version 3 or later",
        "(`LICENSE.GPL-3.0` one folder up). The source for each library is at",
        "the link given; ramus's own source is the matching release tag at",
        "https://github.com/1337raspberry/ramus.",
        "",
        "| Bundled files | Homebrew formula | Version | Licence | Source | Licence files |",
        "| --- | --- | --- | --- | --- | --- |",
        *rows,
        "",
    ]
    (LICENSES_DIR / "NATIVE_LIBRARIES.md").write_text("\n".join(manifest))
    print(f"wrote licences for {len(kegs)} formulae to {LICENSES_DIR}")
    return len(unknown)


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

    # Homebrew bottles are built for the macOS version of the machine that
    # poured them. If that is newer than the app's declared minimum, macOS
    # launches the app on an older system where libmpv then fails to load.
    # The fix is to raise `minimumSystemVersion` or move CI to an older
    # runner image.
    min_system = minimum_system_version()
    too_new: list[tuple[str, str]] = []
    for target_basename in real_to_target.values():
        minos = otool_minos(WORKDIR / target_basename)
        if minos is not None and version_tuple(minos) > version_tuple(min_system):
            too_new.append((target_basename, minos))
    if too_new:
        strict = os.environ.get("CI") == "true"
        label = "ERROR" if strict else "WARN"
        print(
            f"{label}: bundled dylibs target a newer macOS than "
            f"bundle.macOS.minimumSystemVersion ({min_system}):",
            file=sys.stderr,
        )
        for binary, minos in sorted(too_new):
            print(f"  {binary}: minos {minos}", file=sys.stderr)
        if strict:
            return 1
        print(
            "Continuing: this bundle only runs on this Mac's macOS version "
            "or newer.",
            file=sys.stderr,
        )

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
    if collect_licenses(real_to_target) and os.environ.get("CI") == "true":
        print(
            "ERROR: some bundled libraries have no Homebrew formula, so their "
            "licence can't be recorded (see the warnings above).",
            file=sys.stderr,
        )
        return 1

    rel_paths = sorted({f"macos-frameworks/{b}" for b in real_to_target.values()})
    config = {
        "$schema": "https://schema.tauri.app/config/2",
        "bundle": {
            "macOS": {"frameworks": rel_paths},
            # Merged into tauri.conf.json's resources map, beside the
            # project's own licence texts.
            "resources": {"macos-licenses": "licenses/native"},
        },
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
