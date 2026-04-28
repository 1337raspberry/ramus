import { useEffect, useRef } from "react";
import type { Bookmark } from "../lib/types";
import { describeFilters } from "../lib/filterDescribe";
import type { AlbumFilters } from "../stores/libraryStore";

interface Props {
  entries: Bookmark[];
  onSelect: (entry: Bookmark) => void;
  onManage: () => void;
  onDismiss: () => void;
  /// `"sheet"` renders an iOS-style bottom action sheet (mobile).
  /// `"popover"` renders an inline anchored popover (desktop sidebar).
  variant: "sheet" | "popover";
}

/**
 * Picker for 2+ bookmarks. Picking a row applies the bookmark's filter
 * snapshot. The "Manage…" row is always present so editing remains
 * discoverable without relying on the right-click / long-press gesture.
 */
export default function BookmarkPicker({ entries, onSelect, onManage, onDismiss, variant }: Props) {
  const popoverRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (variant !== "popover") return;
    const anchor = popoverRef.current?.closest(".sidebar-bookmarks-anchor");
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
            {entries.length === 0 ? (
              <div className="bookmark-empty-row">
                No bookmarks yet. Set a filter, then tap the … menu in the filter panel to save one.
              </div>
            ) : (
              entries.map((entry) => (
                <button
                  key={entry.id}
                  className="bookmark-sheet-row"
                  onClick={() => {
                    onSelect(entry);
                    onDismiss();
                  }}
                >
                  <span className="bookmark-row-name">{entry.name}</span>
                  <span className="bookmark-row-summary">
                    {describeFilters(filtersFromBookmark(entry))}
                  </span>
                </button>
              ))
            )}
            <button
              onClick={() => {
                onManage();
                onDismiss();
              }}
            >
              Manage bookmarks…
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
    <div className="bookmark-popover" ref={popoverRef}>
      {entries.length === 0 ? (
        <div className="bookmark-empty-row">
          No bookmarks yet. Set a filter, then use the … menu in the filter panel to save one.
        </div>
      ) : (
        entries.map((entry) => (
          <button
            key={entry.id}
            className="bookmark-popover-row"
            onClick={() => {
              onSelect(entry);
              onDismiss();
            }}
            title={describeFilters(filtersFromBookmark(entry))}
          >
            <span className="bookmark-row-name">{entry.name}</span>
            <span className="bookmark-row-summary">
              {describeFilters(filtersFromBookmark(entry))}
            </span>
          </button>
        ))
      )}
      <div className="bookmark-popover-divider" />
      <button
        className="bookmark-popover-row bookmark-popover-manage"
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

/// Project the IPC-shaped `Bookmark.filters` (with `yearMin`/`yearMax`) into
/// the client-side `AlbumFilters` shape (with the raw `year` string) so
/// `describeFilters` can present it. Min/max → "min-max" or single year if
/// equal.
function filtersFromBookmark(b: Bookmark): AlbumFilters {
  const f = b.filters;
  let year = "";
  if (f.yearMin != null && f.yearMax != null) {
    year = f.yearMin === f.yearMax ? `${f.yearMin}` : `${f.yearMin}-${f.yearMax}`;
  } else if (f.yearMin != null) {
    year = `>=${f.yearMin}`;
  } else if (f.yearMax != null) {
    year = `<=${f.yearMax}`;
  }
  return {
    unplayed: f.unplayed,
    favouriteAlbums: f.favouriteAlbums,
    favouriteTracks: f.favouriteTracks,
    year,
    countries: f.countries,
    genres: f.genres,
    collection: f.collection ?? "",
  };
}

export { filtersFromBookmark };
