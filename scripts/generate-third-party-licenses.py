#!/usr/bin/env python3
"""Generate THIRD_PARTY_LICENSES.md from Cargo.lock and ui/package-lock.json.

Runs two tools back-to-back and concatenates their output:

1. `cargo about generate --format json` — walks the Rust dep tree and
   emits machine-readable data. This script re-renders it to markdown
   with strict deterministic ordering (sort by license id, then crate
   name+version). Config lives in about.toml; the `accepted` allowlist
   there doubles as an early warning if a surprise copyleft dep lands.

2. `npx license-checker-rseidelsohn --production --json` — walks
   ui/node_modules (production-only, no devDependencies) and emits
   JSON. Same deterministic re-rendering on this side.

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


def run_license_checker() -> dict:
    """Invoke npm license-checker and parse its JSON output."""
    try:
        result = subprocess.run(
            [
                "npx",
                "--yes",
                "license-checker-rseidelsohn",
                "--production",
                "--json",
                "--relativeLicensePath",
            ],
            cwd=UI,
            check=True,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        print(
            "npx not found. Install Node.js to run license-checker.",
            file=sys.stderr,
        )
        sys.exit(1)
    except subprocess.CalledProcessError as e:
        print("license-checker failed:", file=sys.stderr)
        print(e.stderr, file=sys.stderr)
        sys.exit(1)
    return json.loads(result.stdout)


def read_license_file(license_file: str | None) -> str:
    """Read a license file referenced by license-checker, if accessible.

    `license-checker-rseidelsohn` with --relativeLicensePath returns
    paths relative to cwd (ui/). Returns a short placeholder if the
    file is missing or unreadable; license-checker always supplies the
    SPDX identifier, which is enough for legal attribution even without
    the full text.

    A compromised npm package could point `licenseFile` at a path like
    `../../ramus-tauri/src/...` to slip source-code content into the
    embedded third-party list. The resolved path is confined to `ui/`
    before any read so a hostile manifest can't escape the node_modules
    subtree.
    """
    if not license_file:
        return "(license text not bundled with this package)"
    candidate = UI / license_file
    if not candidate.exists():
        return f"(license text not found at {license_file})"
    ui_root = UI.resolve()
    try:
        resolved = candidate.resolve()
    except OSError as e:
        return f"(could not resolve {license_file}: {e})"
    if ui_root != resolved and ui_root not in resolved.parents:
        return f"(license path {license_file} escapes ui/ — skipped)"
    try:
        return resolved.read_text(encoding="utf-8", errors="replace").strip()
    except OSError as e:
        return f"(could not read {license_file}: {e})"


def format_npm_section(packages: dict) -> str:
    """Render the npm section as canonical markdown.

    license-checker's output is `{"name@version": {...}, ...}`. We sort
    by key for determinism and skip the top-level `ui@*` entry (the app
    itself is not a third-party dep).
    """
    out: list[str] = [
        "## Frontend (npm) dependencies\n",
        "The following npm packages are bundled in ramus's frontend. "
        "Production `dependencies` only — devDependencies like Vite, "
        "TypeScript, and prettier are build-time tooling and are not "
        "shipped in release artifacts.\n",
    ]

    filtered = sorted(
        (k, v) for (k, v) in packages.items() if not k.startswith("ui@")
    )
    for name_ver, info in filtered:
        licenses = info.get("licenses", "UNKNOWN")
        repo = info.get("repository", "")
        license_text = read_license_file(info.get("licenseFile"))
        out.append(f"### {name_ver} ({licenses})\n")
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

    print("Running license-checker (npm package licenses) → JSON...")
    npm_data = run_license_checker()
    npm_md = format_npm_section(npm_data)

    header = (
        "# Third-Party Licenses\n\n"
        "This file lists the open-source components bundled in ramus "
        "release artifacts, along with their license texts. It is "
        "generated by `scripts/generate-third-party-licenses.py` from "
        "`Cargo.lock` and `ui/package-lock.json`. **Do not edit by "
        "hand** — CI diffs this file on every PR and fails if it's "
        "stale; run the script locally and commit the result.\n\n"
        "See `LICENSE` for the ramus license itself (MIT) and "
        "`NOTICE.md` for attribution of bundled data files and "
        "runtime-linked native libraries (libmpv).\n\n"
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
