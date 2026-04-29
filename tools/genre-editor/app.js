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
  dirtyFlag: $("#dirty-flag"),
  search: $("#search"),
  expandAllBtn: $("#expand-all-btn"),
  collapseAllBtn: $("#collapse-all-btn"),
  addRootBtn: $("#add-root-btn"),
  addTsvRootBtn: $("#add-tsv-root-btn"),
  status: $("#status"),
  tree: $("#tree-root"),
  nodeTpl: $("#node-template"),
  akaTpl: $("#aka-template"),
  tsvModal: $("#tsv-modal"),
  tsvTarget: $("#tsv-target"),
  tsvInput: $("#tsv-input"),
  tsvPreview: $("#tsv-preview"),
  tsvAdd: $("#tsv-add"),
  tsvCancel: $("#tsv-cancel"),
};

// Active TSV-modal context: { node, isRoot }. `node` is the parent that new
// children will be appended to; for isRoot=true, children are pushed to
// state.data.genres directly.
let tsvContext = null;

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
  if (!state.path || !state.data) return;
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
  const addTsv = root.querySelector(".add-tsv");
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

  addTsv.addEventListener("click", () => {
    openTsvModal({ node, isRoot: false });
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

// --- TSV paste -------------------------------------------------------------

function parseTsvLines(text) {
  // Format per line: count<tab>name<tab>aka1, aka2, ...
  // Column 1 (count) is ignored. Column 3 is optional. Blank lines are
  // skipped. Lines without a usable name go into `skipped` so the modal can
  // surface them in the live preview.
  const results = [];
  const skipped = [];
  const lines = text.split(/\r?\n/);
  lines.forEach((rawLine, idx) => {
    if (!rawLine.trim()) return;
    const cols = rawLine.split("\t");
    const name = (cols[1] || "").trim();
    const akasRaw = (cols[2] || "").trim();
    if (!name) {
      skipped.push(idx + 1);
      return;
    }
    const aka = akasRaw
      ? akasRaw
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean)
      : [];
    const node = { name, short_summary: null };
    if (aka.length) node.aka = aka;
    node.children = [];
    results.push(node);
  });
  return { results, skipped };
}

function openTsvModal(context) {
  tsvContext = context;
  els.tsvTarget.textContent = context.isRoot ? "(root level)" : context.node.name;
  els.tsvInput.value = "";
  els.tsvPreview.textContent = "";
  els.tsvPreview.className = "modal-preview";
  els.tsvModal.classList.remove("hidden");
  setTimeout(() => els.tsvInput.focus(), 0);
}

function closeTsvModal() {
  els.tsvModal.classList.add("hidden");
  tsvContext = null;
}

function updateTsvPreview() {
  const { results, skipped } = parseTsvLines(els.tsvInput.value);
  if (results.length === 0 && skipped.length === 0) {
    els.tsvPreview.textContent = "";
    els.tsvPreview.className = "modal-preview";
    els.tsvAdd.disabled = true;
    return;
  }
  const wordChild = results.length === 1 ? "child" : "children";
  let msg = `→ ${results.length} ${wordChild} ready`;
  if (skipped.length) {
    const linesWord = skipped.length === 1 ? "line" : "lines";
    msg += `, skipping ${skipped.length} ${linesWord} with no name (line ${skipped.join(", ")})`;
  }
  els.tsvPreview.textContent = msg;
  els.tsvPreview.className =
    "modal-preview " + (skipped.length ? "warn" : results.length ? "ok" : "");
  els.tsvAdd.disabled = results.length === 0;
}

function applyTsvModal() {
  if (!tsvContext) return;
  const { results } = parseTsvLines(els.tsvInput.value);
  if (results.length === 0) {
    setStatus("nothing to add", "err");
    return;
  }
  let statusMsg;
  if (tsvContext.isRoot) {
    state.data.genres.push(...results);
    statusMsg = `added ${results.length} root ${results.length === 1 ? "genre" : "genres"}`;
  } else {
    const target = tsvContext.node;
    target.children.push(...results);
    expandedSet.add(target);
    for (const a of findAncestors(target)) expandedSet.add(a);
    statusMsg = `added ${results.length} ${results.length === 1 ? "child" : "children"} to "${target.name}"`;
  }
  // Flash the first newly-added node so the user can spot the result.
  flashNode = results[0];
  setDirty(true);
  els.fileMeta.textContent = `${countNodes(state.data.genres)} nodes`;
  setStatus(statusMsg, "ok");
  closeTsvModal();
  render();
  flashNode = null;
}

els.tsvInput.addEventListener("input", updateTsvPreview);
els.tsvAdd.addEventListener("click", applyTsvModal);
els.tsvCancel.addEventListener("click", closeTsvModal);
els.tsvModal.addEventListener("click", (e) => {
  // Click on backdrop (outside the card) → close.
  if (e.target === els.tsvModal) closeTsvModal();
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

els.addTsvRootBtn.addEventListener("click", () => {
  openTsvModal({ node: null, isRoot: true });
});

window.addEventListener("keydown", (e) => {
  // Modal-specific shortcuts take precedence.
  if (!els.tsvModal.classList.contains("hidden")) {
    if (e.key === "Escape") {
      e.preventDefault();
      closeTsvModal();
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      applyTsvModal();
      return;
    }
    return;
  }
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
