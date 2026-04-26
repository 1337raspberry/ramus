# genre-editor

local web UI for browsing/editing the genre tree files in `ramus-tauri/data/`
(`open.json`, `edittest.json`, etc).

## run

```sh
python3 tools/genre-editor/server.py
# then open http://127.0.0.1:8765/
```

flags: `--port N` (default 8765), `--host H` (default 127.0.0.1).
no dependencies — stdlib only.

## what it does

- file picker lists every `*.json` in `ramus-tauri/data/`
- cascading expand/collapse tree
- click a name to rename inline (Enter to commit, Esc to cancel)
- `+ aka` adds an editable AKA chip; the small `✕` removes one
- `+ child` adds a child genre (auto-expands and selects the new name)
- `+ root genre` adds at the top level
- `✕` on a row deletes that genre and all descendants (with a confirm)
- drag the `⋮⋮` handle on the left of a row to move a genre (and all its
  children). drop targets:
  - top of a row → insert as previous sibling
  - bottom of a row → insert as next sibling
  - middle of a row → insert as last child (auto-expands the target)
  hovering over a collapsed parent for ~600ms expands it so you can drop deeper.
  in **move mode** (no modifier), cycles are blocked — you can't drop a node
  into itself or a descendant. after a successful drop the moved node is
  scrolled into view and flashes green.
- hold **Ctrl** (Win/Linux), **Cmd** (Mac), or **Alt/Option** while dragging to
  **duplicate** instead of move. the original stays put and a deep clone of the
  whole subtree is inserted at the drop location. the drop indicator turns green
  and the system cursor switches to "+ copy". cycle protection is relaxed in
  copy mode (the clone is a fresh subtree so it's safe to drop into the
  original's descendants — useful for stamping out repeating patterns).
- search box filters by name or AKA, showing the ancestor chain
- Cmd/Ctrl+S saves; "● unsaved" indicator + browser unload warning protect dirty state

## save format

server validates each node has a non-empty `name`, optional string `short_summary`,
optional `aka` list of non-empty strings, and a `children` list. on save it
canonicalises field order (`name`, `short_summary`, `aka`, `children`) and writes
indented JSON with a trailing newline — matching what `merge-aka-into-open-json.py`
produces.

## files

`edittest.json` is a working copy created from `open.json` for safe experimentation;
it is gitignored. when you're happy with edits, `cp edittest.json open.json` (or save
straight to `open.json` from the picker).
