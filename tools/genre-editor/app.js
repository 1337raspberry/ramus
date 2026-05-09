// Genre tree editor — vanilla JS, no build step.
// State: { path, data, dirty }. Tree is mutated in place; DOM mirrors state via re-render.

const state = {
  path: null,
  data: null,
  dirty: false,
};

// Tracks which nodes are expanded so a re-render (e.g. after drag/drop or
// adding a child) preserves the user's view. Keyed by data-node identity.
const expandedSet = new WeakSet();

// Maps a `.node-row` DOM element back to its data node and parent array.
// Used by the drag/drop handlers (delegated on the tree root).
const elToNode = new WeakMap();
const elToParent = new WeakMap();

// Active drag state: { node, parentArr, el } where el is the .node-row.
let dragSrc = null;

// Auto-expand-on-hover: pause briefly over a collapsed node before expanding it
// during a drag, so users can drop into deeper levels.
let hoverRow = null;
let hoverTimer = null;

// One-shot flag: the next render() will scroll to and flash this node.
let flashNode = null;

const $ = (sel) => document.querySelector(sel);
const els = {
  fileSelect: $("#file-select"),
  reloadBtn: $("#reload-btn"),
  fileMeta: $("#file-meta"),
  saveBtn: $("#save-btn"),
  exportTxtBtn: $("#export-txt-btn"),
  importTxtBtn: $("#import-txt-btn"),
  dirtyFlag: $("#dirty-flag"),
  search: $("#search"),
  expandAllBtn: $("#expand-all-btn"),
  collapseAllBtn: $("#collapse-all-btn"),
  addRootBtn: $("#add-root-btn"),
  status: $("#status"),
  tree: $("#tree-root"),
  nodeTpl: $("#node-template"),
  akaTpl: $("#aka-template"),
};

// --- helpers ---------------------------------------------------------------

function setStatus(msg, kind = "") {
  els.status.textContent = msg;
  els.status.className = "status" + (kind ? " " + kind : "");
}

function setDirty(d) {
  state.dirty = d;
  els.dirtyFlag.classList.toggle("hidden", !d);
  els.saveBtn.disabled = !d;
}

function clearChildren(el) {
  el.replaceChildren();
}

function countNodes(genres) {
  let n = 0;
  const walk = (arr) => {
    for (const g of arr) {
      n++;
      if (g.children) walk(g.children);
    }
  };
  walk(genres);
  return n;
}

// --- API ------------------------------------------------------------------

async function loadFiles() {
  const r = await fetch("/api/files");
  const j = await r.json();
  clearChildren(els.fileSelect);
  for (const f of j.files) {
    const opt = document.createElement("option");
    opt.value = f.name;
    opt.textContent = `${f.name} (${(f.size / 1024).toFixed(1)} KB)`;
    els.fileSelect.appendChild(opt);
  }
  // Default to open.json if it's in the list (it's the only checked-in
  // tree today; users may keep extra working copies alongside it).
  const preferred = j.files.find((f) => f.name === "open.json");
  if (preferred) els.fileSelect.value = "open.json";
}

async function loadFile(name) {
  setStatus("loading…");
  const r = await fetch(`/api/file?path=${encodeURIComponent(name)}`);
  if (!r.ok) {
    const j = await r.json().catch(() => ({}));
    setStatus("load failed: " + (j.error || r.status), "err");
    return;
  }
  const j = await r.json();
  state.path = name;
  state.data = j.data;
  // Normalise: ensure children arrays exist.
  const norm = (n) => {
    if (!Array.isArray(n.children)) n.children = [];
    if (n.aka && !Array.isArray(n.aka)) n.aka = [];
    n.children.forEach(norm);
  };
  state.data.genres.forEach(norm);
  setDirty(false);
  els.fileMeta.textContent = `${countNodes(state.data.genres)} nodes`;
  render();
  setStatus("loaded", "ok");
}

async function save() {
  if (!state.data) return;
  if (!state.path) {
    // Imported-but-not-loaded-from-disk case: there's no destination JSON
    // file. The two ways out are export-as-txt or selecting a target file
    // from the dropdown (which would replace the imported tree, so the
    // user shouldn't reach for this expecting to "save as").
    setStatus("imported tree has no destination file — use export .txt, or pick a JSON file from the dropdown to load over", "err");
    return;
  }
  setStatus("saving…");
  const r = await fetch("/api/save", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path: state.path, data: state.data }),
  });
  const j = await r.json();
  if (!r.ok || !j.ok) {
    setStatus("save failed: " + (j.error || r.status), "err");
    return;
  }
  setDirty(false);
  setStatus(`saved ${j.bytes} bytes`, "ok");
  els.fileMeta.textContent = `${countNodes(state.data.genres)} nodes`;
}

// --- export ---------------------------------------------------------------

// Convert the in-memory tree to the indented .txt format that ramus-core's
// CustomGenreParser accepts. Format spec: `Name | aka1 | aka2`, one genre
// per line, indented with two spaces per nesting level. `short_summary`
// is intentionally NOT exported — the parser ignores descriptions, and
// keeping it out of the round-trip avoids the bracket-disambiguation
// edge cases that would otherwise come back to bite us.
function treeToTxt(genres) {
  const lines = [];
  const walk = (node, depth) => {
    const indent = "  ".repeat(depth);
    let line = indent + (node.name || "");
    const akas = (node.aka || [])
      .map((a) => (a || "").trim())
      .filter((a) => a.length > 0);
    if (akas.length) line += " | " + akas.join(" | ");
    lines.push(line);
    for (const c of node.children || []) walk(c, depth + 1);
  };
  for (const root of genres || []) walk(root, 0);
  return lines.join("\n") + "\n";
}

function exportTxt() {
  if (!state.data || !state.data.genres) {
    setStatus("nothing to export", "err");
    return;
  }
  const text = treeToTxt(state.data.genres);
  const base = (state.path || "genres.json").replace(/\.json$/i, "");
  // Browser-native save — anchor with `download` triggers the standard save
  // dialog (Chromium honours user prompts when downloads.always_ask is set).
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = base + ".txt";
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  // Revoke after the navigation has had time to start.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  setStatus(`exported ${countNodes(state.data.genres)} nodes`, "ok");
}

// --- import ---------------------------------------------------------------

// Port of ramus-core/src/genre/parser.rs::CustomGenreParser. Parses the
// indented `Name | aka1 | aka2` text format and returns an editor-shape
// tree (`{name, short_summary: null, aka?, children}`) plus any non-fatal
// warnings. Throws on structural problems (empty file, JSON-looking input,
// indentation jumps, no roots) so the caller can surface a clean error.
function parseTxt(text) {
  if (!text) throw new Error("the file is empty");

  // Strip C0 controls (except tab), DEL, and C1 controls — same set the
  // Rust parser drops in `strip_control_characters`.
  const cleaned = text.replace(/[\x00-\x08\x0b-\x1f\x7f\x80-\x9f]/g, "");

  const rawLines = cleaned.split(/\r?\n/);
  const indexed = []; // { lineNumber, text }
  for (let i = 0; i < rawLines.length; i++) {
    if (rawLines[i].trim()) indexed.push({ lineNumber: i + 1, text: rawLines[i] });
  }
  if (indexed.length === 0) throw new Error("the file is empty");

  const firstChar = indexed[0].text.trim()[0];
  if (firstChar === "{" || firstChar === "[") {
    throw new Error(
      "this looks like JSON or another structured format — use a plain text file with indented genre names instead",
    );
  }

  // Detect indent unit from the first indented line: tab wins over spaces;
  // for spaces, snap to 4 / 2 / 1 based on how deep the first indent goes.
  // Default to 2-space when the file has no indentation at all (a flat
  // list of roots).
  let unit = { kind: "spaces", size: 2 };
  for (const { text: t } of indexed) {
    if (t[0] === "\t") {
      unit = { kind: "tab" };
      break;
    }
    if (t[0] === " ") {
      let n = 0;
      while (t[n] === " ") n++;
      unit = { kind: "spaces", size: n >= 4 ? 4 : n >= 2 ? 2 : 1 };
      break;
    }
  }

  const measure = (line) => {
    if (unit.kind === "tab") {
      let n = 0;
      while (line[n] === "\t") n++;
      return { depth: n, content: line.slice(n).trim() };
    }
    let n = 0;
    while (line[n] === " ") n++;
    const depth = Math.floor(n / unit.size);
    return { depth, content: line.slice(depth * unit.size).trim() };
  };

  const entries = [];
  const warnings = [];
  let prevDepth = 0;

  for (const { lineNumber, text: t } of indexed) {
    const { depth, content } = measure(t);
    const parts = content.split("|");
    const name = (parts[0] || "").trim();
    const akas = parts
      .slice(1)
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    if (!name) {
      warnings.push(`Line ${lineNumber}: skipped — no genre name found.`);
      continue;
    }
    if (depth > prevDepth + 1) {
      throw new Error(
        `line ${lineNumber}: indentation jumps from level ${prevDepth} to ${depth}`,
      );
    }
    prevDepth = depth;
    entries.push({ depth, name, akas, lineNumber });
  }

  if (!entries.some((e) => e.depth === 0)) {
    throw new Error("no root-level genres found");
  }

  // Tree assembly mirrors `build_tree` in the Rust parser: a stack of
  // open-at-depth nodes, popped onto their parent when a sibling/ancestor
  // arrives. Same-level duplicate names produce non-fatal warnings;
  // duplicates across different parents are allowed (cousins can share
  // a name — e.g. "Funk" under both R&B and Pop).
  const roots = [];
  const stack = []; // { depth, node }
  const dupeSets = [new Set()];

  for (const { depth, name, akas, lineNumber } of entries) {
    const newNode = { name, short_summary: null, children: [] };
    if (akas.length) newNode.aka = akas;

    let didPop = false;
    while (stack.length && stack[stack.length - 1].depth >= depth) {
      const popped = stack.pop();
      if (stack.length === 0) roots.push(popped.node);
      else stack[stack.length - 1].node.children.push(popped.node);
      didPop = true;
    }

    if (didPop && depth + 1 < dupeSets.length) {
      dupeSets.length = depth + 1;
    }
    while (dupeSets.length <= depth) dupeSets.push(new Set());

    const key = name.toLowerCase();
    if (dupeSets[depth].has(key)) {
      warnings.push(`Line ${lineNumber}: duplicate genre "${name}" at this level.`);
    } else {
      dupeSets[depth].add(key);
    }

    stack.push({ depth, node: newNode });
  }

  while (stack.length) {
    const last = stack.pop();
    if (stack.length === 0) roots.push(last.node);
    else stack[stack.length - 1].node.children.push(last.node);
  }

  return { genres: roots, warnings };
}

function importTxt() {
  if (state.dirty && !confirm("discard unsaved changes?")) return;
  // Browser-native file picker — same approach as the export anchor, no
  // need for a hidden <input> in the markup.
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".txt,.text,text/plain";
  input.onchange = () => {
    const file = input.files && input.files[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      let parsed;
      try {
        parsed = parseTxt(reader.result || "");
      } catch (err) {
        setStatus("import failed: " + err.message, "err");
        return;
      }
      // Replace the in-memory tree. `state.path` goes null because there
      // is no on-disk JSON file yet — `save()` surfaces that explicitly.
      state.path = null;
      state.data = { genres: parsed.genres };
      // Mirror loadFile()'s normalisation so downstream code can assume
      // children is always an array.
      const norm = (n) => {
        if (!Array.isArray(n.children)) n.children = [];
        n.children.forEach(norm);
      };
      state.data.genres.forEach(norm);

      setDirty(true);
      const total = countNodes(state.data.genres);
      els.fileMeta.textContent = `${total} nodes (imported from ${file.name}, unsaved)`;
      render();

      if (parsed.warnings.length) {
        // Warnings are non-fatal (e.g. same-level duplicates). Surface the
        // count in the status; full list goes to the console so the user
        // can audit without crowding the toolbar.
        console.warn("import warnings:\n" + parsed.warnings.join("\n"));
        const w = parsed.warnings.length;
        setStatus(
          `imported ${total} nodes (${w} warning${w === 1 ? "" : "s"} — see console)`,
          "ok",
        );
      } else {
        setStatus(`imported ${total} nodes`, "ok");
      }
    };
    reader.onerror = () => setStatus("read failed: " + reader.error, "err");
    reader.readAsText(file);
  };
  input.click();
}

// --- rendering ------------------------------------------------------------

function render() {
  clearChildren(els.tree);
  for (const node of state.data.genres) {
    els.tree.appendChild(buildNodeEl(node, state.data.genres));
  }
  applyFilter(els.search.value);
}

function buildNodeEl(node, parentArr) {
  const frag = els.nodeTpl.content.cloneNode(true);
  const root = frag.querySelector(".node");
  const row = root.querySelector(".node-row");
  const toggle = root.querySelector(".toggle");
  const nameEl = root.querySelector(".name");
  const akaList = root.querySelector(".aka-list");
  const addAka = root.querySelector(".add-aka");
  const addChild = root.querySelector(".add-child");
  const del = root.querySelector(".delete");
  const childrenEl = root.querySelector(".children");

  // Register the row so the delegated drag/drop handlers on the tree root
  // can map a DOM event back to its data node and parent array.
  elToNode.set(row, node);
  elToParent.set(row, parentArr);

  nameEl.textContent = node.name || "";
  if (!node.name) nameEl.classList.add("empty");

  const renderAkas = () => {
    clearChildren(akaList);
    for (const a of node.aka || []) {
      akaList.appendChild(buildAkaEl(node, a));
    }
  };
  renderAkas();

  const refreshChildren = () => {
    clearChildren(childrenEl);
    for (const c of node.children) {
      childrenEl.appendChild(buildNodeEl(c, node.children));
    }
    root.classList.toggle("no-children", node.children.length === 0);
  };
  refreshChildren();

  if (expandedSet.has(node)) {
    root.classList.add("expanded");
  }

  if (flashNode === node) {
    // Defer to next tick so the element is actually in the DOM.
    setTimeout(() => {
      root.classList.add("flash");
      root.scrollIntoView({ block: "center", behavior: "smooth" });
      setTimeout(() => root.classList.remove("flash"), 1500);
    }, 0);
  }

  toggle.addEventListener("click", () => {
    if (node.children.length === 0) return;
    if (root.classList.toggle("expanded")) {
      expandedSet.add(node);
    } else {
      expandedSet.delete(node);
    }
  });

  // Inline rename.
  nameEl.addEventListener("blur", () => {
    const t = nameEl.textContent.trim();
    if (t === node.name) return;
    if (!t) {
      nameEl.textContent = node.name;
      setStatus("name cannot be empty", "err");
      return;
    }
    node.name = t;
    nameEl.textContent = t;
    nameEl.classList.remove("empty");
    setDirty(true);
    setStatus("renamed", "ok");
  });
  nameEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      nameEl.blur();
    }
    if (e.key === "Escape") {
      nameEl.textContent = node.name;
      nameEl.blur();
    }
  });

  addAka.addEventListener("click", () => {
    if (!node.aka) node.aka = [];
    node.aka.push("");
    const chip = buildAkaEl(node, "");
    akaList.appendChild(chip);
    setDirty(true);
    // The chip we just appended is the last element child of akaList.
    const last = akaList.lastElementChild;
    const text = last && last.querySelector(".aka-text");
    if (text) text.focus();
  });

  addChild.addEventListener("click", () => {
    const child = { name: "New Genre", short_summary: null, children: [] };
    node.children.push(child);
    setDirty(true);
    expandedSet.add(node);
    root.classList.add("expanded");
    refreshChildren();
    // Focus the new child's name for immediate edit.
    const newRow = childrenEl.lastElementChild;
    const newName = newRow && newRow.querySelector(".name");
    if (newName) {
      newName.focus();
      const range = document.createRange();
      range.selectNodeContents(newName);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
    }
  });

  del.addEventListener("click", () => {
    const n = countSubtree(node);
    const msg =
      n > 1 ? `delete "${node.name}" and ${n - 1} descendant(s)?` : `delete "${node.name}"?`;
    if (!confirm(msg)) return;
    const idx = parentArr.indexOf(node);
    if (idx >= 0) parentArr.splice(idx, 1);
    root.remove();
    setDirty(true);
    setStatus("deleted", "ok");
    els.fileMeta.textContent = `${countNodes(state.data.genres)} nodes`;
  });

  return frag;
}

function buildAkaEl(node, value) {
  const frag = els.akaTpl.content.cloneNode(true);
  const chip = frag.querySelector(".aka-chip");
  const text = chip.querySelector(".aka-text");
  const remove = chip.querySelector(".aka-remove");
  text.textContent = value;
  text.addEventListener("blur", () => {
    const t = text.textContent.trim();
    const idx = (node.aka || []).indexOf(value);
    if (t === value) return;
    if (!t) {
      // Empty AKA → remove.
      if (idx >= 0) node.aka.splice(idx, 1);
      if (node.aka && node.aka.length === 0) delete node.aka;
      chip.remove();
      setDirty(true);
      return;
    }
    if (idx >= 0) {
      node.aka[idx] = t;
      value = t;
      setDirty(true);
    }
  });
  text.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      text.blur();
    }
    if (e.key === "Escape") {
      text.textContent = value;
      text.blur();
    }
  });
  remove.addEventListener("click", () => {
    const idx = (node.aka || []).indexOf(value);
    if (idx >= 0) node.aka.splice(idx, 1);
    if (node.aka && node.aka.length === 0) delete node.aka;
    chip.remove();
    setDirty(true);
  });
  return frag;
}

function countSubtree(node) {
  let n = 1;
  for (const c of node.children || []) n += countSubtree(c);
  return n;
}

// --- search/filter --------------------------------------------------------

function applyFilter(q) {
  q = q.trim().toLowerCase();
  if (!q) {
    // No filter: full re-render so all nodes are visible.
    clearChildren(els.tree);
    for (const node of state.data.genres) {
      els.tree.appendChild(buildNodeEl(node, state.data.genres));
    }
    return;
  }
  // Walk the data tree; mark nodes whose name/aka match. Show match + ancestors.
  const matchSet = new WeakSet();
  const walk = (node, ancestors) => {
    const hay = (node.name || "").toLowerCase() + " " + (node.aka || []).join(" ").toLowerCase();
    const isMatch = hay.includes(q);
    if (isMatch) {
      matchSet.add(node);
      for (const a of ancestors) matchSet.add(a);
    }
    for (const c of node.children || []) walk(c, [...ancestors, node]);
  };
  for (const g of state.data.genres) walk(g, []);
  rebuildWithFilter(matchSet, q);
}

function rebuildWithFilter(matchSet, q) {
  clearChildren(els.tree);
  const lc = q.toLowerCase();
  const build = (node, parentArr, container) => {
    if (!matchSet.has(node)) return;
    container.appendChild(buildNodeEl(node, parentArr));
    const nodeEl = container.lastElementChild;
    if (q) {
      if (node.children.some((c) => matchSet.has(c))) nodeEl.classList.add("expanded");
      const hay =
        (node.name || "").toLowerCase() + " " + (node.aka || []).join(" ").toLowerCase();
      if (hay.includes(lc)) nodeEl.querySelector(".node-row").classList.add("match");
    }
    const childContainer = nodeEl.querySelector(".children");
    clearChildren(childContainer);
    for (const c of node.children) {
      build(c, node.children, childContainer);
    }
  };
  for (const g of state.data.genres) build(g, state.data.genres, els.tree);
}

// --- drag and drop --------------------------------------------------------

function nodeContains(parent, target) {
  // True if `target` is `parent` or anywhere in its subtree. Used to block
  // moves that would create a cycle (drop into self / descendant).
  if (parent === target) return true;
  for (const c of parent.children || []) if (nodeContains(c, target)) return true;
  return false;
}

function findAncestors(target) {
  // Walks the data tree and returns the chain of ancestor nodes leading to
  // `target` (root-first, target excluded). Used to expand the path so a
  // moved node is visible after re-render.
  const path = [];
  const walk = (nodes, chain) => {
    for (const n of nodes) {
      if (n === target) {
        path.push(...chain);
        return true;
      }
      if (walk(n.children || [], [...chain, n])) return true;
    }
    return false;
  };
  walk(state.data.genres, []);
  return path;
}

function computeDropZone(row, clientY) {
  const rect = row.getBoundingClientRect();
  const y = clientY - rect.top;
  const h = rect.height;
  if (y < h * 0.28) return "before";
  if (y > h * 0.72) return "after";
  return "inside";
}

function clearDropIndicators() {
  document
    .querySelectorAll(".drop-before, .drop-after, .drop-inside, .drop-copy")
    .forEach((el) => {
      el.classList.remove("drop-before", "drop-after", "drop-inside", "drop-copy");
    });
}

function clearHoverTimer() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  hoverRow = null;
}

function isCopyDrag(e) {
  // Hold Ctrl (Win/Linux), Cmd (Mac), or Alt/Option to duplicate instead of move.
  return e.ctrlKey || e.metaKey || e.altKey;
}

function deepCloneNode(node) {
  // Genre nodes are plain JSON (name/short_summary/aka/children), so a
  // structured deep copy via JSON round-trip is safe and gives every node a
  // fresh identity (so expandedSet entries for the original aren't carried
  // over to the clone).
  return JSON.parse(JSON.stringify(node));
}

function isDropAllowed(targetNode, copy) {
  if (!dragSrc) return false;
  if (copy) {
    // Copy mode: anywhere is fair game. The clone is a fresh subtree, so
    // dropping into the original's descendants (or itself) doesn't form a cycle.
    return true;
  }
  if (targetNode === dragSrc.node) return false;
  // Any drop relative to a descendant of the dragged node would re-parent
  // it under itself (cycle) or under one of its own children.
  if (nodeContains(dragSrc.node, targetNode)) return false;
  return true;
}

els.tree.addEventListener("dragstart", (e) => {
  const handle = e.target.closest(".drag-handle");
  if (!handle) {
    // Drag started somewhere that isn't the handle (e.g. the contenteditable
    // name) — block it so we don't end up dragging text into the tree.
    e.preventDefault();
    return;
  }
  const row = handle.closest(".node-row");
  const node = elToNode.get(row);
  const parentArr = elToParent.get(row);
  if (!row || !node || !parentArr) {
    e.preventDefault();
    return;
  }
  dragSrc = { node, parentArr, el: row };
  row.parentNode.classList.add("dragging");
  // Allow either op — `dragover` decides between them based on modifier keys.
  e.dataTransfer.effectAllowed = "copyMove";
  // Some browsers require any data to start a drag.
  e.dataTransfer.setData("text/plain", node.name || "");
});

els.tree.addEventListener("dragend", () => {
  if (dragSrc) dragSrc.el.parentNode.classList.remove("dragging");
  clearDropIndicators();
  clearHoverTimer();
  dragSrc = null;
});

els.tree.addEventListener("dragover", (e) => {
  if (!dragSrc) return;
  const row = e.target.closest(".node-row");
  if (!row) return;
  const targetNode = elToNode.get(row);
  if (!targetNode) return;
  const zone = computeDropZone(row, e.clientY);
  const copy = isCopyDrag(e);
  if (!isDropAllowed(targetNode, copy)) {
    clearDropIndicators();
    clearHoverTimer();
    return;
  }
  e.preventDefault();
  e.dataTransfer.dropEffect = copy ? "copy" : "move";
  clearDropIndicators();
  row.classList.add("drop-" + zone);
  if (copy) row.classList.add("drop-copy");

  // Auto-expand collapsed nodes after a brief hover so users can drop deeper.
  if (row !== hoverRow) {
    clearHoverTimer();
    hoverRow = row;
    if (
      zone === "inside" &&
      targetNode.children.length > 0 &&
      !expandedSet.has(targetNode)
    ) {
      hoverTimer = setTimeout(() => {
        expandedSet.add(targetNode);
        row.parentNode.classList.add("expanded");
      }, 600);
    }
  }
});

els.tree.addEventListener("dragleave", (e) => {
  // Only clear when the cursor truly leaves the tree (relatedTarget is the
  // element being entered — null means leaving the document/window).
  if (!e.relatedTarget || !els.tree.contains(e.relatedTarget)) {
    clearDropIndicators();
    clearHoverTimer();
  }
});

els.tree.addEventListener("drop", (e) => {
  if (!dragSrc) return;
  const row = e.target.closest(".node-row");
  if (!row) return;
  const targetNode = elToNode.get(row);
  const targetParent = elToParent.get(row);
  if (!targetNode || !targetParent) return;
  const zone = computeDropZone(row, e.clientY);
  const copy = isCopyDrag(e);
  if (!isDropAllowed(targetNode, copy)) return;

  e.preventDefault();

  // For copy: clone now, leave the original in place. For move: lift the
  // node out of its current parent first, then insert at the target.
  let srcIdx = -1;
  if (!copy) {
    srcIdx = dragSrc.parentArr.indexOf(dragSrc.node);
    if (srcIdx < 0) return;
    dragSrc.parentArr.splice(srcIdx, 1);
  }
  const nodeToInsert = copy ? deepCloneNode(dragSrc.node) : dragSrc.node;

  // Insert at the target.
  if (zone === "inside") {
    targetNode.children.push(nodeToInsert);
    expandedSet.add(targetNode);
  } else {
    let tIdx = targetParent.indexOf(targetNode);
    if (tIdx < 0) {
      // Should never happen — restore the moved node and bail.
      if (!copy) dragSrc.parentArr.splice(srcIdx, 0, dragSrc.node);
      return;
    }
    if (zone === "after") tIdx++;
    targetParent.splice(tIdx, 0, nodeToInsert);
  }

  // Expand the chain of ancestors so the new/moved node is visible.
  for (const a of findAncestors(nodeToInsert)) expandedSet.add(a);

  flashNode = nodeToInsert;
  const name = nodeToInsert.name;
  setDirty(true);
  setStatus(`${copy ? "duplicated" : "moved"} "${name}"`, "ok");
  els.fileMeta.textContent = `${countNodes(state.data.genres)} nodes`;

  // Clear active drag state before re-render (the row element is about to be
  // replaced by render()).
  clearDropIndicators();
  clearHoverTimer();
  dragSrc = null;

  render();
  // flashNode is consumed once buildNodeEl matched it; clear so subsequent
  // renders don't re-flash.
  flashNode = null;
});

// --- wire up --------------------------------------------------------------

function setAllExpanded(expanded) {
  // Update the set so subsequent re-renders preserve this state.
  const walk = (nodes) => {
    for (const n of nodes) {
      if ((n.children || []).length > 0) {
        if (expanded) expandedSet.add(n);
        else expandedSet.delete(n);
      }
      walk(n.children || []);
    }
  };
  walk(state.data.genres);
  els.tree.querySelectorAll(".node").forEach((el) => {
    if (expanded) el.classList.add("expanded");
    else el.classList.remove("expanded");
  });
}

els.fileSelect.addEventListener("change", () => {
  if (state.dirty && !confirm("discard unsaved changes?")) {
    els.fileSelect.value = state.path;
    return;
  }
  loadFile(els.fileSelect.value);
});

els.reloadBtn.addEventListener("click", () => {
  if (state.dirty && !confirm("discard unsaved changes?")) return;
  loadFile(els.fileSelect.value);
});

els.saveBtn.addEventListener("click", save);
els.exportTxtBtn.addEventListener("click", exportTxt);
els.importTxtBtn.addEventListener("click", importTxt);

els.search.addEventListener("input", () => {
  applyFilter(els.search.value);
});

els.expandAllBtn.addEventListener("click", () => setAllExpanded(true));
els.collapseAllBtn.addEventListener("click", () => setAllExpanded(false));

els.addRootBtn.addEventListener("click", () => {
  const root = { name: "New Root Genre", short_summary: null, children: [] };
  state.data.genres.push(root);
  setDirty(true);
  els.tree.appendChild(buildNodeEl(root, state.data.genres));
  const lastNode = els.tree.lastElementChild;
  const name = lastNode && lastNode.querySelector(".name");
  if (name) {
    name.focus();
    const range = document.createRange();
    range.selectNodeContents(name);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }
  if (lastNode) lastNode.scrollIntoView({ block: "center" });
});

window.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    if (!els.saveBtn.disabled) save();
  }
});

window.addEventListener("beforeunload", (e) => {
  if (state.dirty) {
    e.preventDefault();
    e.returnValue = "";
  }
});

// --- boot -----------------------------------------------------------------

(async () => {
  try {
    await loadFiles();
    if (els.fileSelect.value) {
      await loadFile(els.fileSelect.value);
    } else {
      setStatus("no .json files found in data dir", "err");
    }
  } catch (err) {
    setStatus("init failed: " + err.message, "err");
    console.error(err);
  }
})();
