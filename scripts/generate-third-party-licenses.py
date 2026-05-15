#!/usr/bin/env python3
"""Generate THIRD_PARTY_LICENSES.md from Cargo.lock and ui/pnpm-lock.yaml.

Runs two tools back-to-back and concatenates their output:

1. `cargo about generate --format json` — walks the Rust dep tree and
   emits machine-readable data. This script re-renders it to markdown
   with strict deterministic ordering (sort by license id, then crate
   name+version). Config lives in config/about.toml; the `accepted` allowlist
   there doubles as an early warning if a surprise copyleft dep lands.

2. `pnpm licenses list --prod --json` — walks ui/node_modules
   (production-only, no devDependencies) and emits JSON. Same
   deterministic re-rendering on this side. Was `npx
   license-checker-rseidelsohn` before the pnpm migration; the npx form
   fetched a remote tool on every run, exactly the supply-chain pattern
   the migration was meant to harden against.

Output is written to /THIRD_PARTY_LICENSES.md at the repo root, plus a
copy at /licenses/THIRD_PARTY_LICENSES.md so the bundled licenses
directory is self-contained for Tauri's bundle.resources.

CI runs this script on every PR and diffs the output against the
committed copy; drift fails the build with a loud remediation message
(see .github/workflows/ci.yml). Must be deterministic — no timestamps,
no HashMap-ordered sections. Handlebars templates in cargo-about are
not deterministic across runs (identical runs can group crates under
different sections based on internal HashMap order), which is why this
script goes via JSON and does its own canonical rendering.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parent
UI = ROOT / "ui"
OUT = ROOT / "THIRD_PARTY_LICENSES.md"
BUNDLED_COPY = ROOT / "licenses" / "THIRD_PARTY_LICENSES.md"


def run_cargo_about_json() -> dict:
    """Invoke cargo-about and capture its JSON output.

    `--frozen` (= `--locked --offline`) is required for deterministic
    output. Without it, cargo-about enriches license metadata via
    ClearlyDefined (clearlydefined.io), which can return different
    data across runs for crates that are still being "harvested" —
    producing a byte-different markdown each run and making the CI
    drift check useless. Offline mode falls back to local Cargo.toml
    license fields and on-disk LICENSE files, which is sufficient for
    attribution; the only thing we lose is copyright-line enrichment
    pulled from upstream git repositories, which wasn't worth the
    flake.
    """
    try:
        result = subprocess.run(
            [
                "cargo",
                "about",
                "generate",
                "--format",
                "json",
                "--frozen",
                "--manifest-path",
                "Cargo.toml",
                "--config",
                "config/about.toml",
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        print(
            "cargo-about not found. Install with: "
            "cargo install cargo-about --locked",
            file=sys.stderr,
        )
        sys.exit(1)
    except subprocess.CalledProcessError as e:
        print("cargo about generate --format json failed:", file=sys.stderr)
        print(e.stderr, file=sys.stderr)
        sys.exit(1)
    return json.loads(result.stdout)


def format_rust_section(data: dict) -> str:
    """Render the Rust section as canonical markdown.

    Groups by license SPDX id, lists crates alphabetically within each
    section, and dedupes by (license_id, stripped license text) so
    byte-identical texts collapse to one block. If a license id has
    multiple distinct texts (rare — happens for Apache-2.0 with tiny
    whitespace differences), each distinct text gets its own subsection
    sorted by the first crate that uses it.

    Everything sorts by plain string comparison so the output is stable
    across runs and machines.
    """
    # Collect: license_id -> dict(text -> list of (crate_name, version, repo))
    grouped: dict[str, dict[str, list[tuple[str, str, str]]]] = defaultdict(
        lambda: defaultdict(list)
    )
    license_display_name: dict[str, str] = {}

    for lic in data.get("licenses", []):
        lic_id = lic.get("id") or "UNKNOWN"
        license_display_name[lic_id] = lic.get("name") or lic_id
        # Strip trailing whitespace so texts that differ only by a trailing
        # newline or spaces dedupe into one block.
        text = (lic.get("text") or "").strip()
        for entry in lic.get("used_by", []):
            crate = entry.get("crate", {})
            name = crate.get("name", "?")
            version = crate.get("version", "?")
            repo = crate.get("repository") or ""
            grouped[lic_id][text].append((name, version, repo))

    out: list[str] = [
        "## Rust dependencies\n",
        "The following Rust crates are bundled in ramus binaries. "
        "Each section lists the crates using a given license, "
        "followed by the license text.\n",
    ]

    for lic_id in sorted(grouped.keys()):
        display = license_display_name[lic_id]
        for text_idx, (text, crates) in enumerate(
            sorted(grouped[lic_id].items(), key=lambda kv: _sort_key_for_text(kv))
        ):
            # If there's only one text for this license id, omit the
            # (variant N) suffix; otherwise include it so multiple
            # blocks with the same license name remain distinguishable.
            suffix = ""
            if len(grouped[lic_id]) > 1:
                suffix = f" (variant {text_idx + 1})"
            out.append(f"### {display} ({lic_id}){suffix}\n")
            out.append("Used by:")
            for name, version, repo in sorted(crates):
                if repo:
                    out.append(f"- `{name}` {version} — {repo}")
                else:
                    out.append(f"- `{name}` {version}")
            out.append("")
            out.append("```")
            out.append(text if text else "(no license text available)")
            out.append("```\n")

    return "\n".join(out) + "\n"


def _sort_key_for_text(
    kv: tuple[str, list[tuple[str, str, str]]]
) -> tuple[str, str]:
    """Sort key for license-text variants within a single license id.

    Orders by the first (alphabetically earliest) crate name that uses
    that text, so `(variant 1)` is always the one with the
    alphabetically-first crate. Stable across runs.
    """
    _text, crates = kv
    first_crate = min(c[0] for c in crates) if crates else ""
    return (first_crate, _text[:100])


def run_pnpm_licenses() -> list[dict]:
    """Invoke `pnpm licenses list` and flatten to one record per (name, version).

    Output shape from pnpm is `{license_id: [{name, versions, paths, ...}, ...]}`
    — packages with multiple resolved versions in the tree are collapsed under
    one entry with parallel `versions` / `paths` arrays. We flatten back to one
    record per (name, version) so the markdown renderer can sort and dedupe
    deterministically.

    Switched from `npx license-checker-rseidelsohn` (which fetched a remote
    package on every run, and didn't handle pnpm's symlinked node_modules
    layout cleanly) to pnpm's built-in command after the npm migration.
    """
    try:
        result = subprocess.run(
            ["pnpm", "licenses", "list", "--prod", "--json"],
            cwd=UI,
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        print(
            "pnpm not found. Install pnpm to enumerate npm dep licenses "
            "(brew install pnpm).",
            file=sys.stderr,
        )
        sys.exit(1)
    except subprocess.CalledProcessError as e:
        print("pnpm licenses list failed:", file=sys.stderr)
        print(e.stderr, file=sys.stderr)
        sys.exit(1)
    grouped = json.loads(result.stdout)
    flat: list[dict] = []
    for license_id, entries in grouped.items():
        for entry in entries:
            name = entry.get("name", "?")
            versions = entry.get("versions") or []
            paths = entry.get("paths") or []
            # Parallel arrays: zip what we have, padding the shorter side.
            for i in range(max(len(versions), len(paths), 1)):
                version = versions[i] if i < len(versions) else "?"
                pkg_path = paths[i] if i < len(paths) else None
                flat.append(
                    {
                        "name": name,
                        "version": version,
                        "license": license_id,
                        "path": pkg_path,
                        "repository": _read_repo(pkg_path),
                    }
                )
    return flat


def _read_repo(pkg_path: str | None) -> str:
    """Extract the `repository` field from a package's package.json."""
    if not pkg_path:
        return ""
    pj = Path(pkg_path) / "package.json"
    if not pj.exists():
        return ""
    try:
        data = json.loads(pj.read_text(encoding="utf-8", errors="replace"))
    except (OSError, json.JSONDecodeError):
        return ""
    repo = data.get("repository")
    if isinstance(repo, str):
        return repo
    if isinstance(repo, dict):
        return repo.get("url", "") or ""
    return ""


# Filenames a package may use for its license text. Walked in order; the
# first match wins. Covers both `LICENSE` and `LICENCE` spellings and the
# three common extensions, in upper/lower/mixed case to match what packages
# actually ship.
LICENSE_FILENAMES = (
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "LICENCE",
    "LICENCE.md",
    "LICENCE.txt",
    "License",
    "License.md",
    "License.txt",
    "license",
    "license.md",
    "license.txt",
)


def find_license_text(pkg_path: str | None) -> str:
    """Locate and read a package's LICENSE file from its install path.

    pnpm's `licenses list` doesn't carry the license-text path the way the
    old license-checker tool did, so we walk the package's directory
    ourselves. The path is confined to `ui/node_modules/` before reading so
    a hostile package.json that somehow points outside its install dir
    can't be coerced into slipping unrelated content into the output.
    """
    if not pkg_path:
        return "(license text not bundled with this package)"
    pkg_dir = Path(pkg_path)
    if not pkg_dir.exists():
        return f"(package directory not found: {pkg_path})"
    ui_root = UI.resolve()
    try:
        resolved = pkg_dir.resolve()
    except OSError as e:
        return f"(could not resolve {pkg_path}: {e})"
    if ui_root != resolved and ui_root not in resolved.parents:
        return f"(license path {pkg_path} escapes ui/ — skipped)"
    for name in LICENSE_FILENAMES:
        candidate = pkg_dir / name
        if candidate.exists():
            try:
                return candidate.read_text(
                    encoding="utf-8", errors="replace"
                ).strip()
            except OSError as e:
                return f"(could not read {name}: {e})"
    return "(no LICENSE file found in package)"


def format_npm_section(packages: list[dict]) -> str:
    """Render the npm section as canonical markdown.

    Sorted by (name, version) so the output is byte-stable across runs.
    Skips the top-level `ui` package (the app itself, not a third-party
    dep) and dedupes by (name, version) — pnpm can emit the same package
    multiple times if it resolves to the same version through different
    dependency graph paths.
    """
    out: list[str] = [
        "## Frontend (npm) dependencies\n",
        "The following npm packages are bundled in ramus's frontend. "
        "Production `dependencies` only — devDependencies like Vite, "
        "TypeScript, and prettier are build-time tooling and are not "
        "shipped in release artifacts.\n",
    ]

    seen: set[tuple[str, str]] = set()
    deduped: list[dict] = []
    for pkg in packages:
        if pkg["name"] == "ui":
            continue
        key = (pkg["name"], pkg["version"])
        if key in seen:
            continue
        seen.add(key)
        deduped.append(pkg)

    for pkg in sorted(deduped, key=lambda p: (p["name"], p["version"])):
        name_ver = f"{pkg['name']}@{pkg['version']}"
        license_text = find_license_text(pkg.get("path"))
        repo = pkg.get("repository") or ""
        out.append(f"### {name_ver} ({pkg['license']})\n")
        if repo:
            out.append(f"Repository: {repo}\n")
        out.append("```")
        out.append(license_text if license_text else "(no license text)")
        out.append("```\n")

    return "\n".join(out) + "\n"


def main() -> int:
    print("Running cargo-about (Rust crate licenses) → JSON...")
    rust_data = run_cargo_about_json()
    rust_md = format_rust_section(rust_data)

    print("Running pnpm licenses list (npm package licenses) → JSON...")
    npm_data = run_pnpm_licenses()
    npm_md = format_npm_section(npm_data)

    header = (
        "# Third-Party Licenses\n\n"
        "This file lists the open-source components bundled in ramus "
        "release artifacts, along with their license texts. It is "
        "generated by `scripts/generate-third-party-licenses.py` from "
        "`Cargo.lock` and `ui/pnpm-lock.yaml`. **Do not edit by "
        "hand** — CI diffs this file on every PR and fails if it's "
        "stale; run the script locally and commit the result.\n\n"
        "See `LICENSE` for the ramus license itself (MIT) and "
        "`licenses/NOTICE.md` for attribution of bundled data files and "
        "runtime-linked native libraries (libmpv on every platform, plus "
        "the supporting libraries — ffmpeg, libplacebo, libass, etc — "
        "shipped inside the Android AAR).\n\n"
        "---\n\n"
    )

    combined = header + rust_md + "\n---\n\n" + npm_md
    OUT.write_text(combined, encoding="utf-8")
    print(f"wrote {OUT} ({len(combined)} bytes)")

    BUNDLED_COPY.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(OUT, BUNDLED_COPY)
    print(f"copied to {BUNDLED_COPY}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
