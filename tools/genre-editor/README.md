# genre-editor

local web UI for browsing/editing the genre tree files in `ramus-tauri/data/`
(`open.json`, plus any extra working copies you keep alongside it).

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
- `+ tsv` (per row) and `+ tsv roots` (toolbar) open a paste modal: drop in
  tab-separated rows of the form `count⇥name⇥aka1, aka2, …` and each non-empty
  line becomes a child of that node (or a new root). column 1 (count) is
  ignored, column 3 (AKAs) is optional. live preview shows how many will be
  added and which lines were skipped (no usable name). ⌘/Ctrl+Enter to commit,
  Esc or click outside the card to cancel.
- `+ root genre` adds a single empty root at the top level
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

`open.json` is the only checked-in tree. if you want to experiment without
clobbering it, `cp open.json scratch.json` and select `scratch.json` from the
picker; the editor lists every `*.json` in `ramus-tauri/data/`. when you're
happy with the result, save back to `open.json`.
