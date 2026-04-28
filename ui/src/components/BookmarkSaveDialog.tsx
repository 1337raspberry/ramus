import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useSettingsStore } from "../stores/settingsStore";
import { useLibraryStore, filtersToIPC } from "../stores/libraryStore";
import { isNameUnique, makeBookmark } from "../lib/bookmark";
import { MAX_BOOKMARKS } from "../lib/types";
import { describeFilters } from "../lib/filterDescribe";
import { useToastStore } from "./Toast";

interface Props {
  onDismiss: () => void;
}

/**
 * Modal that captures a name for a new bookmark from the active filter
 * snapshot. Triggered from `FilterPanelMenu.handleBookmark`. The dialog
 * shows a plain-English summary of what's being saved so the user can see
 * exactly what the bookmark will match before naming it.
 */
export default function BookmarkSaveDialog({ onDismiss }: Props) {
  const filters = useLibraryStore((s) => s.albumFilters);
  const bookmarks = useSettingsStore((s) => s.bookmarks);
  const setBookmarks = useSettingsStore((s) => s.setBookmarks);

  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

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

  const summary = useMemo(() => describeFilters(filters), [filters]);

  const trimmed = name.trim();
  const duplicate = trimmed.length > 0 && !isNameUnique(bookmarks, trimmed);
  const atCap = bookmarks.length >= MAX_BOOKMARKS;
  const valid = trimmed.length > 0 && !duplicate && !atCap;

  const handleSave = async () => {
    if (!valid) return;
    setSaving(true);
    setError(null);
    try {
      const bookmark = makeBookmark(trimmed, filtersToIPC(filters));
      await setBookmarks([...bookmarks, bookmark]);
      useToastStore.getState().show("Bookmark saved");
      onDismiss();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void handleSave();
  };

  const handleBackdrop = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  return createPortal(
    <div className="settings-backdrop" onClick={handleBackdrop}>
      <div className="settings-panel glass bookmark-save-dialog">
        <div className="settings-header">
          <h2>Save bookmark</h2>
          <button className="settings-close" onClick={onDismiss}>
            x
          </button>
        </div>

        <form className="settings-body" onSubmit={handleSubmit}>
          {error && (
            <div className="settings-error" onClick={() => setError(null)}>
              {error}
            </div>
          )}

          <div className="bookmark-save-summary-label">Filter snapshot</div>
          <div className="bookmark-save-summary">{summary}</div>

          <label className="bookmark-save-name-label" htmlFor="bookmark-save-name">
            Name
          </label>
          <input
            id="bookmark-save-name"
            ref={inputRef}
            className="bookmark-save-input"
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Study Albums or Workout Favs"
            maxLength={60}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
          {!saving && duplicate && (
            <div className="bookmark-edit-error">Name &ldquo;{trimmed}&rdquo; is already used.</div>
          )}
          {!saving && atCap && (
            <div className="bookmark-edit-error">
              Maximum of {MAX_BOOKMARKS} bookmarks reached. Delete one from the bookmarks editor
              first.
            </div>
          )}

          <div className="bookmark-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="bookmark-btn" onClick={onDismiss}>
              Cancel
            </button>
            <button
              type="submit"
              className="bookmark-btn bookmark-save"
              disabled={!valid || saving}
            >
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body,
  );
}
