import { useEffect, useMemo, useState } from "react";
import { useSettingsStore } from "../stores/settingsStore";
import { isNameUnique, makeSavedSearch } from "../lib/savedSearch";
import { MAX_SAVED_SEARCHES, type SavedSearch } from "../lib/types";

interface Props {
  onDismiss: () => void;
}

interface DraftRow {
  id: string;
  name: string;
  query: string;
}

/**
 * Modal list editor for saved searches. Works on both desktop and mobile
 * via the shared `.settings-panel` + `.saved-search-*` class palette.
 *
 * Edits are held in local component state and committed on Save to avoid
 * persisting transient half-broken lists (e.g. during rename typing where
 * two rows briefly share a name) and to minimise settings.json writes.
 */
export default function SavedSearchEditor({ onDismiss }: Props) {
  const persisted = useSettingsStore((s) => s.savedSearches);
  const setSavedSearches = useSettingsStore((s) => s.setSavedSearches);

  const [rows, setRows] = useState<DraftRow[]>(() =>
    persisted.map((s) => ({ id: s.id, name: s.name, query: s.query })),
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

  const addRow = () => {
    if (rows.length >= MAX_SAVED_SEARCHES) return;
    const draft = makeSavedSearch("", "");
    setRows((prev) => [...prev, { id: draft.id, name: "", query: "" }]);
  };

  const deleteRow = (id: string) => {
    setRows((prev) => prev.filter((r) => r.id !== id));
  };

  const { valid, sanitised } = useMemo(() => {
    const sanitisedRows: SavedSearch[] = [];
    let ok = true;
    for (const r of rows) {
      const query = r.query.trim();
      const name = r.name.trim() || query;
      if (!query) {
        ok = false;
        continue;
      }
      if (!isNameUnique(sanitisedRows, name)) {
        ok = false;
        continue;
      }
      sanitisedRows.push({ id: r.id, name, query });
    }
    return { valid: ok && sanitisedRows.length === rows.length, sanitised: sanitisedRows };
  }, [rows]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await setSavedSearches(sanitised);
      onDismiss();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const atCap = rows.length >= MAX_SAVED_SEARCHES;

  return (
    <div className="settings-backdrop" onClick={handleBackdrop}>
      <div className="settings-panel glass saved-search-editor">
        <div className="settings-header">
          <h2>Saved searches</h2>
          <button className="settings-close" onClick={onDismiss}>
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
            <div className="downloads-empty">No saved searches yet.</div>
          ) : (
            <ul className="saved-search-edit-list">
              {rows.map((r) => {
                const query = r.query.trim();
                const name = r.name.trim() || query;
                // Project draft rows into their effective-name shape so
                // the shared `isNameUnique` helper applies the same rule
                // the store does at save time.
                const effective = rows.map((o) => ({
                  id: o.id,
                  name: o.name.trim() || o.query.trim(),
                  query: o.query,
                }));
                const duplicate = !!name && !isNameUnique(effective, name, r.id);
                const missingQuery = !query;
                return (
                  <li key={r.id} className="saved-search-edit-row">
                    <div className="saved-search-edit-fields">
                      <input
                        className="saved-search-edit-input"
                        type="text"
                        value={r.name}
                        onChange={(e) => updateRow(r.id, { name: e.target.value })}
                        placeholder={query || "Name"}
                        autoComplete="off"
                        autoCorrect="off"
                        autoCapitalize="off"
                        spellCheck={false}
                      />
                      <input
                        className="saved-search-edit-input saved-search-edit-query"
                        type="text"
                        value={r.query}
                        onChange={(e) => updateRow(r.id, { query: e.target.value })}
                        placeholder="/genre @artist %album !track #>year col:name"
                        autoComplete="off"
                        autoCorrect="off"
                        autoCapitalize="off"
                        spellCheck={false}
                      />
                      {(duplicate || missingQuery) && (
                        <div className="saved-search-edit-error">
                          {missingQuery ? "Query required." : `Name "${name}" is already used.`}
                        </div>
                      )}
                    </div>
                    <button
                      className="downloads-row-action"
                      onClick={() => deleteRow(r.id)}
                      title="Remove saved search"
                    >
                      x
                    </button>
                  </li>
                );
              })}
            </ul>
          )}

          <button className="settings-btn" onClick={addRow} disabled={atCap}>
            + Add saved search
          </button>
          {atCap && (
            <div className="saved-search-hint">Maximum of {MAX_SAVED_SEARCHES} reached.</div>
          )}
          <div className="saved-search-hint">
            Queries use the same operators as search: <code>/genre</code>, <code>@artist</code>,
            <code>%album</code>, <code>!track</code>, <code>#&gt;year</code>, <code>col:name</code>,
            joined with <code> AND </code>. Blank name defaults to the query itself.
          </div>

          <div className="saved-search-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="saved-search-btn" onClick={onDismiss}>
              Cancel
            </button>
            <button
              type="button"
              className="saved-search-btn saved-search-save"
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
