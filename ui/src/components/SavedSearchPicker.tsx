import { useEffect, useRef } from "react";
import type { SavedSearch } from "../lib/types";

interface Props {
  entries: SavedSearch[];
  onSelect: (entry: SavedSearch) => void;
  onManage: () => void;
  onDismiss: () => void;
  /// `"sheet"` renders an iOS-style bottom action sheet (mobile).
  /// `"popover"` renders an inline anchored popover (desktop sidebar).
  variant: "sheet" | "popover";
}

/**
 * Picker for 2+ saved searches. Left-click/tap shows this; picking a row
 * loads it. The "Manage…" row is always present so discoverability of
 * the editor doesn't depend on the user remembering the right-click /
 * long-press gesture.
 */
export default function SavedSearchPicker({
  entries,
  onSelect,
  onManage,
  onDismiss,
  variant,
}: Props) {
  const popoverRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (variant !== "popover") return;
    // Outside-click dismiss. Tests against the nearest
    // `.sidebar-saved-anchor` wrapper (which includes the trigger button)
    // so clicking the anchor itself doesn't immediately reopen after the
    // mousedown-before-click race. Matches the `album-card-menu-wrap`
    // pattern used elsewhere.
    const anchor = popoverRef.current?.closest(".sidebar-saved-anchor");
    if (!anchor) return;
    const handler = (e: MouseEvent) => {
      if (!anchor.contains(e.target as Node)) onDismiss();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [variant, onDismiss]);

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

  if (variant === "sheet") {
    return (
      <div
        className="mobile-action-sheet-backdrop"
        onClick={(e) => {
          if (e.target === e.currentTarget) onDismiss();
        }}
      >
        <div className="mobile-action-sheet">
          <div className="mobile-action-sheet-group">
            {entries.map((entry) => (
              <button
                key={entry.id}
                onClick={() => {
                  onSelect(entry);
                  onDismiss();
                }}
              >
                {entry.name}
              </button>
            ))}
            <button
              onClick={() => {
                onManage();
                onDismiss();
              }}
            >
              Manage saved searches…
            </button>
          </div>
          <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
            Cancel
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="saved-search-popover" ref={popoverRef}>
      {entries.map((entry) => (
        <button
          key={entry.id}
          className="saved-search-popover-row"
          onClick={() => {
            onSelect(entry);
            onDismiss();
          }}
          title={entry.query}
        >
          {entry.name}
        </button>
      ))}
      <div className="saved-search-popover-divider" />
      <button
        className="saved-search-popover-row saved-search-popover-manage"
        onClick={() => {
          onManage();
          onDismiss();
        }}
      >
        Manage…
      </button>
    </div>
  );
}
