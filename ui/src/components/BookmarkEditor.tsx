import { useEffect, useMemo, useState } from "react";
import { useSettingsStore } from "../stores/settingsStore";
import { filtersFromBookmark, isNameUnique } from "../lib/bookmark";
import { MAX_BOOKMARKS, type Bookmark } from "../lib/types";
import { describeFilters } from "../lib/filterDescribe";

interface Props {
  onDismiss: () => void;
}

interface DraftRow {
  id: string;
  name: string;
  filters: Bookmark["filters"];
}

/**
 * Modal list editor for bookmarks. Edits are held in local component state
 * and committed on Save to avoid persisting transient half-broken lists
 * (e.g. during rename typing where two rows briefly share a name) and to
 * minimise settings.json writes.
 *
 * Bookmarks are recreated from the filter UI rather than edited in-place,
 * so this editor only renames and deletes — there's no filter-editing
 * surface here.
 */
export default function BookmarkEditor({ onDismiss }: Props) {
  const persisted = useSettingsStore((s) => s.bookmarks);
  const setBookmarks = useSettingsStore((s) => s.setBookmarks);

  const [rows, setRows] = useState<DraftRow[]>(() =>
    persisted.map((b) => ({ id: b.id, name: b.name, filters: b.filters })),
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDismiss]);

  const handleBackdrop = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  const updateRow = (id: string, patch: Partial<DraftRow>) => {
    setRows((prev) => prev.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  };

  const deleteRow = (id: string) => {
    setRows((prev) => prev.filter((r) => r.id !== id));
  };

  const { valid, sanitised } = useMemo(() => {
    const sanitisedRows: Bookmark[] = [];
    let ok = true;
    for (const r of rows) {
      const name = r.name.trim();
      if (!name) {
        ok = false;
        continue;
      }
      if (!isNameUnique(sanitisedRows, name)) {
        ok = false;
        continue;
      }
      sanitisedRows.push({ id: r.id, name, filters: r.filters });
    }
    return { valid: ok && sanitisedRows.length === rows.length, sanitised: sanitisedRows };
  }, [rows]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await setBookmarks(sanitised);
      onDismiss();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings-backdrop" onClick={handleBackdrop}>
      <div
        className="settings-panel glass bookmark-editor"
        role="dialog"
        aria-modal="true"
        aria-labelledby="bookmark-editor-title"
      >
        <div className="settings-header">
          <h2 id="bookmark-editor-title">Bookmarks</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            x
          </button>
        </div>

        <div className="settings-body">
          {error && (
            <div className="settings-error" onClick={() => setError(null)}>
              {error}
            </div>
          )}

          {rows.length === 0 ? (
            <div className="downloads-empty">
              No bookmarks yet — set a filter and use the … menu in the filter panel to save one.
            </div>
          ) : (
            <ul className="bookmark-edit-list">
              {rows.map((r) => {
                const name = r.name.trim();
                const effective = rows.map((o) => ({
                  id: o.id,
                  name: o.name.trim(),
                  filters: o.filters,
                }));
                const duplicate = !!name && !isNameUnique(effective, name, r.id);
                const missingName = !name;
                const summary = describeFilters(
                  filtersFromBookmark({ id: r.id, name: r.name, filters: r.filters }),
                );
                return (
                  <li key={r.id} className="bookmark-edit-row">
                    <div className="bookmark-edit-fields">
                      <input
                        className="bookmark-edit-input"
                        type="text"
                        value={r.name}
                        onChange={(e) => updateRow(r.id, { name: e.target.value })}
                        placeholder="Bookmark name"
                        autoComplete="off"
                        autoCorrect="off"
                        autoCapitalize="off"
                        spellCheck={false}
                      />
                      <div className="bookmark-edit-summary">{summary}</div>
                      {(duplicate || missingName) && (
                        <div className="bookmark-edit-error">
                          {missingName ? "Name required." : `Name "${name}" is already used.`}
                        </div>
                      )}
                    </div>
                    <button
                      className="downloads-row-action"
                      onClick={() => deleteRow(r.id)}
                      title="Remove bookmark"
                    >
                      x
                    </button>
                  </li>
                );
              })}
            </ul>
          )}

          <div className="bookmark-hint">
            To create or update a bookmark, set a filter and use the … menu in the filter panel.
            Bookmarks store the filter, not the results, so newly added matching albums show up
            automatically.
          </div>
          {rows.length >= MAX_BOOKMARKS && (
            <div className="bookmark-hint">Maximum of {MAX_BOOKMARKS} bookmarks reached.</div>
          )}

          <div className="bookmark-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="bookmark-btn" onClick={onDismiss}>
              Cancel
            </button>
            <button
              type="button"
              className="bookmark-btn bookmark-save"
              onClick={handleSave}
              disabled={!valid || saving}
            >
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
