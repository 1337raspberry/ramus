import { useEffect, useRef, useState } from "react";

interface Props {
  initialQuery: string;
  onSave: (query: string) => void;
  onClear?: () => void;
  onDismiss: () => void;
}

export default function SavedSearchModal({ initialQuery, onSave, onClear, onDismiss }: Props) {
  const [query, setQuery] = useState(initialQuery);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleBackdrop = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = query.trim();
    if (trimmed) onSave(trimmed);
  };

  return (
    <div className="saved-search-backdrop" onClick={handleBackdrop}>
      <form className="saved-search-modal" onSubmit={handleSubmit}>
        <div className="saved-search-title">Saved Search</div>
        <input
          ref={inputRef}
          className="saved-search-input"
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="/genre @artist %album !track #>year col:name"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
        <div className="saved-search-hint">
          Uses the same operators as search. Albums matching the query will load when you tap the
          brain icon.
        </div>
        <div className="saved-search-actions">
          {onClear && (
            <button type="button" className="saved-search-btn saved-search-clear" onClick={onClear}>
              Clear
            </button>
          )}
          <div style={{ flex: 1 }} />
          <button type="button" className="saved-search-btn" onClick={onDismiss}>
            Cancel
          </button>
          <button
            type="submit"
            className="saved-search-btn saved-search-save"
            disabled={!query.trim()}
          >
            Save
          </button>
        </div>
      </form>
    </div>
  );
}
