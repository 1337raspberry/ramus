# genre-editor

Local web UI for browsing/editing the genre tree files in `ramus-tauri/data/`
(`open.json`, plus any extra working copies you keep alongside it).

Use it to tweak the bundled tree, build your own from scratch, or share trees
with other people via plain `.txt` files that round-trip cleanly with ramus's
in-app importer (Settings → Genres → Import).

---

## usage

### prerequisites

You need **Python 3** on your machine. No third-party packages — the server
uses only the standard library.

- **macOS** — Python 3 is preinstalled on recent versions. Check with
  `python3 --version`. If you need a newer one: `brew install python`, or
  grab an installer from [python.org](https://www.python.org/downloads/).
- **Windows** — install from
  [python.org](https://www.python.org/downloads/) and tick **"Add python.exe
  to PATH"** during setup. Verify in PowerShell with `python --version`.
- **Linux** — almost certainly already installed. If not,
  `sudo apt install python3` (Debian/Ubuntu) or your distro's equivalent.

### running

From the repo root:

```sh
python3 tools/genre-editor/server.py
```

Then open <http://127.0.0.1:8765/> in any browser.

Flags:

- `--port N` (default `8765`)
- `--host H` (default `127.0.0.1`)

### the basics

- The dropdown at the top lists every `*.json` file under `ramus-tauri/data/`.
  `open.json` is the bundled tree — pick that, or any other, to start editing.
- Click a name to rename it inline. **Enter** to commit, **Esc** to cancel.
- `+ child` adds a child under the selected row; `+ root genre` adds a new
  top-level genre.
- `✕` on a row deletes that genre and everything beneath it (with a
  confirmation prompt).
- `+ aka` adds an "also known as" chip; click the small `✕` on a chip to
  remove one. AKAs are alternate names for the same genre — useful when
  different libraries tag the same thing different ways.
- The search box filters by name or AKA and shows the full ancestor chain
  so you don't lose context.
- **Cmd/Ctrl+S** saves your changes back to the chosen JSON file. An
  "● unsaved" indicator and a browser-unload warning protect dirty state.

### moving things around

Drag the `⋮⋮` handle on the left of a row to move a genre (and all its
children). Drop targets:

- top of a row → insert as the previous sibling
- bottom of a row → insert as the next sibling
- middle of a row → insert as a child of that row (auto-expands)

Hovering over a collapsed parent for ~600ms during a drag expands it so you
can drop deeper. After a successful drop the moved node scrolls into view
and flashes green.

In **move mode** (no modifier), cycles are blocked — you can't drop a node
into itself or one of its descendants.

Hold **Ctrl** (Windows/Linux), **Cmd** (Mac), or **Alt/Option** while
dragging to **duplicate** instead of move. The original stays put and a deep
clone of the whole subtree is inserted at the drop location; the drop
indicator turns green and the system cursor switches to "+ copy". Cycle
protection is relaxed in copy mode — the clone is a fresh subtree, so it's
safe to drop into the original's descendants. Handy for stamping out
repeating patterns.

### sharing trees with other people

Two toolbar buttons:

- **`export .txt`** — downloads the current in-memory tree (including
  unsaved edits) as a plain `.txt` file. The format is exactly what ramus's
  in-app importer accepts, so you can hand the file to someone else, paste
  it in a gist, etc. AKAs are preserved.
- **`import .txt`** — the inverse. Pick a `.txt` file in the same format
  and it replaces the current in-memory tree so you can edit it here. An
  imported tree has no destination JSON file, so save is effectively
  disabled — pick a file from the dropdown to save into, or export back to
  `.txt`.

A typical share flow:

1. Build / refine your tree in the editor.
2. `export .txt` and send the file to someone.
3. They drop it into ramus via Settings → Genres → Import (or import it
   into the editor here, edit further, and re-export).

---

## technical details

### .txt format

Plain indented list, one genre per line:

```
Name | aka1 | aka2
```

- Indent with **2 spaces** or **1 tab** per nesting level — the parser
  auto-detects on the first indented line.
- AKAs are pipe-separated and optional; empty pipe segments are dropped.
- Bracket characters (`[`, `]`) have no special meaning — they read as
  ordinary name/AKA characters, so AKAs like `Hardcore [EDM]` pass through
  verbatim.
- Descriptions (`short_summary`) are **not** part of the `.txt` round-trip;
  edit them directly in the JSON if you need to preserve them.
- Same-level duplicate names produce console warnings on import.
- No JSON-shaped input — the parser expects the indented text format only.

### json save format

The server validates each node has a non-empty `name`, an optional string
`short_summary`, an optional `aka` list of non-empty strings, and a
`children` list. On save it canonicalises field order
(`name`, `short_summary`, `aka`, `children`) and writes indented JSON with a
trailing newline — matching what `scripts/merge-aka-into-open-json.py`
produces, so diffs stay clean.

### files

`open.json` is the only tree checked into the repo. If you want to
experiment without clobbering it:

```sh
cp ramus-tauri/data/open.json ramus-tauri/data/scratch.json
```

The picker lists every `*.json` in `ramus-tauri/data/`, so `scratch.json`
will show up immediately. When you're happy with the result, save back to
`open.json`.

### server scope

The Python server is a tiny stdlib-only HTTP server intended for **local
use only**. It binds to `127.0.0.1` by default, has no auth, and can write
to any `*.json` under `ramus-tauri/data/` that the dropdown surfaces. Don't
expose it to your LAN with `--host 0.0.0.0` unless you trust everyone on it.
