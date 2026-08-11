import { useCallback, useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import {
  SORT_FIELDS,
  defaultDirectionFor,
  sortLabelFor,
  type AlbumSortField,
} from "../lib/albumSort";
import { IconChevronDown, IconShuffle } from "./Icons";

/**
 * Sort control for the album grid header: a button showing the current sort
 * plus a popover listing every field. Tapping the active row flips its
 * direction (or reshuffles, for Random); tapping another row switches to it
 * with that field's default direction. The popover stays open across taps so
 * a direction flip is a single extra click.
 */
export default function SortDropdown() {
  const albumSort = useLibraryStore((s) => s.albumSort);
  const setAlbumSort = useLibraryStore((s) => s.setAlbumSort);
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Element | null;
      if (!target || !wrapRef.current) return;
      if (wrapRef.current.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const handleSelect = useCallback(
    (field: AlbumSortField) => {
      if (field === albumSort.field) {
        if (field === "random") {
          // Re-selecting Random reshuffles.
          setAlbumSort({ ...albumSort });
        } else {
          setAlbumSort({
            field,
            direction: albumSort.direction === "asc" ? "desc" : "asc",
          });
        }
      } else {
        setAlbumSort({ field, direction: defaultDirectionFor(field) });
      }
    },
    [albumSort, setAlbumSort],
  );

  return (
    <div className="sort-dropdown-wrap" ref={wrapRef}>
      <button
        className="filter-dropdown-btn sort-dropdown-btn"
        onClick={() => setOpen((v) => !v)}
        title="Sort albums"
      >
        <span>{sortLabelFor(albumSort.field)}</span>
        {albumSort.field === "random" ? (
          <IconShuffle size={11} />
        ) : (
          <IconChevronDown
            size={10}
            className={`sort-dir-chevron${albumSort.direction === "asc" ? " asc" : ""}`}
          />
        )}
      </button>
      {open && (
        <div className="sort-dropdown-panel">
          {SORT_FIELDS.map(({ field, label }) => {
            const active = field === albumSort.field;
            return (
              <button
                key={field}
                className={`sort-option${active ? " active" : ""}`}
                onClick={() => handleSelect(field)}
              >
                <span>{label}</span>
                {active &&
                  (field === "random" ? (
                    <IconShuffle size={13} />
                  ) : (
                    <IconChevronDown
                      size={12}
                      className={`sort-dir-chevron${albumSort.direction === "asc" ? " asc" : ""}`}
                    />
                  ))}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
