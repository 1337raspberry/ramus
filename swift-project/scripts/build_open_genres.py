#!/usr/bin/env python3
"""
Build open.json from the beets genres-tree.yaml (MIT licensed).

Source: https://github.com/beetbox/beets/blob/master/beetsplug/lastgenre/genres-tree.yaml

Outputs:
  {"updated_at": "...", "genres": [{"name": "...", "short_summary": null, "children": [...]}]}
"""

import json
import os
import re
import sqlite3
import sys
import urllib.request
from datetime import datetime, timezone

BEETS_URL = "https://raw.githubusercontent.com/beetbox/beets/refs/heads/master/beetsplug/lastgenre/genres-tree.yaml"
USER_AGENT = "RamusGenreBuilder/1.0 (https://github.com/1337raspberry/ramus)"

# Words/patterns to keep as-is when title-casing (acronyms, special forms)
_PRESERVE_UPPER = {
    "DJ", "EBM", "EDM", "IDM", "MPB", "NYC", "R&B", "UK", "US",
    "NRG", "J-POP", "K-POP", "J-ROCK", "K-ROCK", "C-POP",
}


def _title_case(name: str) -> str:
    """Title-case a genre name, matching the Rust title_case() logic.

    - Lowercase words get first letter capitalised
    - Words with existing uppercase are preserved (e.g., "EBM", "R&B")
    - Hyphenated compounds are handled per segment
    """
    def case_word(w: str) -> str:
        if w.upper() in _PRESERVE_UPPER:
            return w.upper()
        if w == w.lower():
            return w.capitalize()
        return w  # Has mixed/upper case already — preserve

    parts = name.split(" ")
    result = []
    for part in parts:
        if "-" in part:
            result.append("-".join(case_word(seg) for seg in part.split("-")))
        else:
            result.append(case_word(part))
    return " ".join(result)


def fetch_yaml(cache_path: str | None = None) -> str:
    """Download beets genres-tree.yaml, or read from local cache."""
    if cache_path and os.path.exists(cache_path):
        print(f"Reading cached YAML: {cache_path}")
        with open(cache_path) as f:
            return f.read()

    print(f"Downloading beets genres-tree.yaml...")
    req = urllib.request.Request(BEETS_URL, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=30) as resp:
        content = resp.read().decode("utf-8")

    if cache_path:
        with open(cache_path, "w") as f:
            f.write(content)
        print(f"  Cached to: {cache_path}")

    return content


def parse_yaml_tree(content: str) -> list[dict]:
    """Parse the beets YAML into our JSON tree format.

    The beets YAML is a simple indented structure:
      - genre_name:
          - child_genre
          - child_with_kids:
              - grandchild

    We use a simple line-based parser instead of pulling in PyYAML.
    """
    lines = content.splitlines()
    # Stack: list of (indent_level, node_dict)
    # We build the tree by tracking indentation
    root_children: list[dict] = []
    stack: list[tuple[int, list[dict]]] = [(-1, root_children)]

    for line in lines:
        stripped = line.rstrip()
        if not stripped or stripped.startswith("#"):
            continue

        # Calculate indent (number of leading spaces)
        indent = len(line) - len(line.lstrip())

        # Extract name: "- genre_name:" or "- genre_name"
        text = stripped.lstrip("- ").rstrip(":")
        if not text:
            continue

        node = {
            "name": _title_case(text),
            "short_summary": None,
            "children": [],
        }

        # Pop stack until we find the right parent level
        while len(stack) > 1 and stack[-1][0] >= indent:
            stack.pop()

        # Add to current parent's children
        stack[-1][1].append(node)

        # Push this node's children list as the new context
        stack.append((indent, node["children"]))

    return root_children


def apply_overrides(tree: list[dict], overrides_path: str) -> list[dict]:
    """Apply manual overrides from genre_overrides.json.

    Supported keys:
      force_remove: list of genre names to delete (case-insensitive)
      rename: dict of old_name -> new_name
      force_description: dict of name -> description
    """
    if not os.path.exists(overrides_path):
        return tree

    with open(overrides_path) as f:
        overrides = json.load(f)

    if not any(overrides.get(k) for k in ("force_remove", "rename", "force_description")):
        return tree

    print(f"Applying overrides from {os.path.basename(overrides_path)}...")

    remove_set = {n.lower() for n in overrides.get("force_remove", [])}
    rename_map = {k.lower(): v for k, v in overrides.get("rename", {}).items()}
    desc_map = {k.lower(): v for k, v in overrides.get("force_description", {}).items()}

    def process(nodes: list[dict]) -> list[dict]:
        result = []
        for n in nodes:
            key = n["name"].lower()
            if key in remove_set:
                print(f"  Removed: {n['name']}")
                continue
            if key in rename_map:
                print(f"  Renamed: {n['name']} → {rename_map[key]}")
                n["name"] = rename_map[key]
            if key in desc_map:
                n["short_summary"] = desc_map[key]
            if n.get("children"):
                n["children"] = process(n["children"])
            result.append(n)
        return result

    return process(tree)


def count_genres(nodes: list[dict]) -> int:
    """Count total genres in the tree."""
    total = 0
    for n in nodes:
        total += 1
        if n.get("children"):
            total += count_genres(n["children"])
    return total


def extract_names(nodes: list[dict], names: set[str] | None = None) -> set[str]:
    """Extract all genre names from the tree."""
    if names is None:
        names = set()
    for n in nodes:
        names.add(n["name"].lower())
        if n.get("children"):
            extract_names(n["children"], names)
    return names


def match_report(tree: list[dict]) -> None:
    """Print a match report against the user's Plex genres if the cache DB exists."""
    db_path = os.path.expanduser("~/.local/share/ramus/cache.db")
    if not os.path.exists(db_path):
        print("\n  Cache DB not found, skipping Plex match report")
        return

    conn = sqlite3.connect(db_path)
    plex_genres = [row[0] for row in conn.execute("SELECT name FROM genres ORDER BY name")]
    conn.close()

    plex_lower = set(g.lower() for g in plex_genres)
    tree_names = extract_names(tree)

    matched = plex_lower & tree_names
    unmatched = sorted(plex_lower - tree_names)

    print(f"\n=== Plex Library Match Report ===")
    print(f"  Plex genres: {len(plex_lower)}")
    print(f"  Open.json genres: {len(tree_names)}")
    print(f"  Exact matches: {len(matched)}/{len(plex_lower)} ({len(matched)/len(plex_lower)*100:.1f}%)")
    print(f"  Unmatched: {len(unmatched)}")
    if unmatched:
        print(f"  First 30 unmatched:")
        for g in unmatched[:30]:
            print(f"    × {g}")
        if len(unmatched) > 30:
            print(f"    ... and {len(unmatched) - 30} more")


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    cache_path = os.path.join(script_dir, "genres-tree.yaml")

    # Fetch / read the beets YAML
    content = fetch_yaml(cache_path)

    # Parse into tree
    print("Parsing genre tree...")
    tree = parse_yaml_tree(content)

    # Apply overrides
    overrides_path = os.path.join(script_dir, "genre_overrides.json")
    tree = apply_overrides(tree, overrides_path)

    total = count_genres(tree)
    print(f"Final tree: {len(tree)} top-level categories, {total} total genres")

    # Output
    output = {
        "updated_at": datetime.now(timezone.utc).isoformat(),
        "genres": tree,
    }

    # Write to both Swift and Tauri paths
    project_root = os.path.dirname(script_dir)
    xplat_root = os.path.dirname(project_root)
    output_paths = [
        os.path.join(project_root, "ramus", "Resources", "open.json"),
        os.path.join(xplat_root, "ramus-tauri", "data", "open.json"),
    ]

    for output_path in output_paths:
        if os.path.exists(os.path.dirname(output_path)):
            with open(output_path, "w") as f:
                json.dump(output, f, indent=2, ensure_ascii=False)
            print(f"\nWritten to: {output_path}")
            print(f"File size: {os.path.getsize(output_path) / 1024:.1f} KB")

    # Match report
    match_report(tree)


if __name__ == "__main__":
    main()
