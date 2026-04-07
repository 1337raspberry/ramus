#!/usr/bin/env python3
"""
Fetch music genre data from Wikidata (CC0) and build open.json
for the Ramus genre tree.

Outputs the same JSON format as genres.json:
  {"updated_at": "...", "genres": [{"name": "...", "short_summary": null, "children": [...]}]}

Names are kept exactly as Wikidata provides them — no normalization.
"""

import json
import os
import sqlite3
import sys
import urllib.parse
import urllib.request
from collections import defaultdict
from datetime import datetime, timezone

SPARQL_ENDPOINT = "https://query.wikidata.org/sparql"
USER_AGENT = "RamusGenreBuilder/1.0 (https://github.com/1337raspberry/ramus)"

# Broad super-categories to group root genres under.
# Keys are category names, values are keyword sets to match against genre labels.
SUPER_CATEGORIES = {
    "rock music": {"rock", "grunge", "britpop", "shoegaze", "surf"},
    "electronic music": {"electronic", "electro", "techno", "house", "trance", "dubstep",
                         "drum and bass", "breakbeat", "edm", "synth", "chiptune",
                         "ambient", "downtempo", "idm"},
    "jazz": {"jazz", "bebop", "swing", "bossa nova", "bop"},
    "classical music": {"classical", "symphony", "concerto", "sonata", "opera", "baroque",
                        "chamber", "orchestral", "choral", "cantata", "oratorio",
                        "romantic music", "minimalism"},
    "hip hop music": {"hip hop", "hip-hop", "rap", "rapping", "trap"},
    "heavy metal music": {"metal", "doom", "thrash", "death metal", "black metal",
                          "grindcore", "metalcore"},
    "pop music": {"pop", "bubblegum", "boy band", "girl group", "teen"},
    "folk music": {"folk", "traditional", "ballad"},
    "blues": {"blues"},
    "country music": {"country", "bluegrass", "honky-tonk", "outlaw"},
    "rhythm and blues": {"r&b", "soul", "funk", "motown", "doo-wop", "gospel"},
    "reggae": {"reggae", "ska", "dancehall", "dub", "rocksteady", "calypso"},
    "punk rock": {"punk", "hardcore"},
    "dance music": {"dance", "disco"},
    "world music": {"world", "afro", "latin", "samba", "flamenco", "fado", "tango",
                    "cumbia", "merengue", "salsa", "bossa", "mariachi"},
    "experimental music": {"experimental", "avant-garde", "noise", "drone",
                           "musique concrète", "sound art", "free improvisation"},
    "new-age music": {"new age", "new-age", "meditation"},
    "spoken word": {"spoken word", "poetry", "audiobook", "comedy"},
    "musical theatre": {"musical theatre", "musical theater", "show tunes", "cabaret"},
    "religious music": {"religious", "sacred", "hymn", "gospel", "liturgical",
                        "devotional", "christian", "qawwali", "bhajan", "kirtan"},
}


def sparql_query(query: str) -> list[dict]:
    """Execute a SPARQL query against Wikidata and return bindings."""
    url = SPARQL_ENDPOINT + "?" + urllib.parse.urlencode({
        "format": "json",
        "query": query,
    })
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = json.loads(resp.read())
    return data["results"]["bindings"]


def fetch_genres() -> dict[str, dict]:
    """Fetch all music genres with parent relationships from Wikidata."""
    print("Fetching genres from Wikidata SPARQL endpoint...")

    query = """
    SELECT ?genre ?genreLabel ?genreDescription ?parent ?parentLabel WHERE {
      ?genre wdt:P31 wd:Q188451 .
      OPTIONAL { ?genre wdt:P279 ?parent . ?parent wdt:P31 wd:Q188451 . }
      SERVICE wikibase:label { bd:serviceParam wikibase:language "en" . }
    }
    """

    bindings = sparql_query(query)
    print(f"  Received {len(bindings)} rows")

    genres: dict[str, dict] = {}
    for b in bindings:
        qid = b["genre"]["value"].split("/")[-1]
        label = b["genreLabel"]["value"]

        # Skip genres whose label is just the QID (no English label)
        if label.startswith("Q") and label[1:].isdigit():
            continue

        if qid not in genres:
            desc = b.get("genreDescription", {}).get("value") or None
            genres[qid] = {"label": label, "description": desc, "parents": [], "children": []}
        elif genres[qid]["description"] is None:
            desc = b.get("genreDescription", {}).get("value") or None
            if desc:
                genres[qid]["description"] = desc

        if "parent" in b:
            parent_qid = b["parent"]["value"].split("/")[-1]
            parent_label = b["parentLabel"]["value"]
            # Skip parents with no English label
            if parent_label.startswith("Q") and parent_label[1:].isdigit():
                continue
            if parent_qid not in [p["qid"] for p in genres[qid]["parents"]]:
                genres[qid]["parents"].append({"qid": parent_qid, "label": parent_label})

    print(f"  Unique genres: {len(genres)}")
    return genres


def build_tree(genres: dict[str, dict]) -> list[dict]:
    """Convert the DAG into a tree by choosing canonical parents."""

    # Build child count per genre (how many genres list it as a parent)
    child_count: dict[str, int] = defaultdict(int)
    for g in genres.values():
        for p in g["parents"]:
            if p["qid"] in genres:
                child_count[p["qid"]] += 1

    # For multi-parent genres, pick the parent with the most children (most canonical)
    parent_map: dict[str, str | None] = {}  # qid -> parent_qid or None
    for qid, g in genres.items():
        valid_parents = [p for p in g["parents"] if p["qid"] in genres]
        if not valid_parents:
            parent_map[qid] = None
        elif len(valid_parents) == 1:
            parent_map[qid] = valid_parents[0]["qid"]
        else:
            # Pick parent with most children (most "canonical" category)
            best = max(valid_parents, key=lambda p: child_count.get(p["qid"], 0))
            parent_map[qid] = best["qid"]

    # Build children lists
    children_of: dict[str, list[str]] = defaultdict(list)
    roots: list[str] = []
    for qid, parent_qid in parent_map.items():
        if parent_qid is None:
            roots.append(qid)
        else:
            children_of[parent_qid].append(qid)

    print(f"  Root genres (no parent): {len(roots)}")
    print(f"  Genres with parents: {len(genres) - len(roots)}")

    # Build tree nodes recursively with cycle protection
    def build_node(qid: str, visited: set[str]) -> dict | None:
        if qid in visited:
            return None  # Cycle detected, skip
        visited.add(qid)

        g = genres[qid]
        child_nodes = []
        for child_qid in sorted(children_of.get(qid, []),
                                 key=lambda q: genres[q]["label"].lower()):
            node = build_node(child_qid, visited)
            if node:
                child_nodes.append(node)

        visited.discard(qid)

        return {
            "name": g["label"],
            "short_summary": g.get("description"),
            "children": child_nodes if child_nodes else [],
        }

    # Group roots under super-categories
    categorized: dict[str, list[str]] = defaultdict(list)
    uncategorized: list[str] = []

    for qid in roots:
        label = genres[qid]["label"].lower()
        matched = False
        for cat_name, keywords in SUPER_CATEGORIES.items():
            for kw in keywords:
                if kw in label:
                    categorized[cat_name].append(qid)
                    matched = True
                    break
            if matched:
                break
        if not matched:
            uncategorized.append(qid)

    print(f"  Categorized roots: {sum(len(v) for v in categorized.values())}")
    print(f"  Uncategorized roots: {len(uncategorized)}")

    # Build top-level tree
    top_level: list[dict] = []

    # Super-category nodes
    for cat_name in sorted(categorized.keys()):
        cat_children = []
        for qid in sorted(categorized[cat_name],
                          key=lambda q: genres[q]["label"].lower()):
            node = build_node(qid, set())
            if node:
                cat_children.append(node)

        # If the super-category name itself is a genre in our data,
        # use it as the parent and add its own Wikidata children too
        cat_qid = None
        for qid, g in genres.items():
            if g["label"].lower() == cat_name.lower() and parent_map.get(qid) is not None:
                cat_qid = qid
                break

        if cat_qid and cat_qid in children_of:
            # Merge: the category genre's own children + the root genres assigned here
            own_node = build_node(cat_qid, set())
            if own_node:
                # Deduplicate children by name
                existing_names = {c["name"] for c in own_node["children"]}
                for child in cat_children:
                    if child["name"] not in existing_names:
                        own_node["children"].append(child)
                own_node["children"].sort(key=lambda c: c["name"].lower())
                top_level.append(own_node)
                continue

        top_level.append({
            "name": cat_name,
            "short_summary": None,
            "children": cat_children,
        })

    # Uncategorized roots — all added as top-level entries.
    # No "Other" bucket in the data; the runtime buildDisplayTree handles
    # unmatched Plex genres in its own "Other" node.
    for qid in sorted(uncategorized, key=lambda q: genres[q]["label"].lower()):
        node = build_node(qid, set())
        if node:
            top_level.append(node)

    top_level.sort(key=lambda n: n["name"].lower())
    return top_level


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
    # Fetch data
    genres = fetch_genres()

    # Build tree
    print("\nBuilding tree...")
    tree = build_tree(genres)

    total = count_genres(tree)
    print(f"\nFinal tree: {len(tree)} top-level categories, {total} total genres")

    # Output
    output = {
        "updated_at": datetime.now(timezone.utc).isoformat(),
        "genres": tree,
    }

    # Determine output path
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(script_dir)
    output_path = os.path.join(project_root, "ramus", "Resources", "open.json")

    with open(output_path, "w") as f:
        json.dump(output, f, indent=2, ensure_ascii=False)

    print(f"\nWritten to: {output_path}")
    print(f"File size: {os.path.getsize(output_path) / 1024:.1f} KB")

    # Match report
    match_report(tree)


if __name__ == "__main__":
    main()
