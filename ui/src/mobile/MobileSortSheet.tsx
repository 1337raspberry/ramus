import { createPortal } from "react-dom";
import { useLibraryStore } from "../stores/libraryStore";
import { SORT_FIELDS, defaultDirectionFor, type AlbumSortField } from "../lib/albumSort";
import { IconChevronDown, IconShuffle } from "../components/Icons";

/**
 * Bottom sheet listing every sort field. Tapping the active row flips its
 * direction (Random reshuffles instead); tapping another row switches to it
 * with that field's default direction. The sheet stays open across taps so
 * flipping direction doesn't mean reopening it — dismissed by Close or the
 * backdrop.
 */
export default function MobileSortSheet({ onDismiss }: { onDismiss: () => void }) {
  const albumSort = useLibraryStore((s) => s.albumSort);
  const setAlbumSort = useLibraryStore((s) => s.setAlbumSort);

  const handleSelect = (field: AlbumSortField) => {
    if (field === albumSort.field) {
      if (field === "random") {
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
  };

  return createPortal(
    <div
      className="mobile-action-sheet-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="mobile-action-sheet">
        <div className="mobile-action-sheet-group">
          <div className="mobile-action-sheet-header">Sort albums</div>
          {SORT_FIELDS.map(({ field, label }) => {
            const active = field === albumSort.field;
            return (
              <button
                key={field}
                className={`mobile-sort-row${active ? " active" : ""}`}
                onClick={() => handleSelect(field)}
              >
                <span>{label}</span>
                {active &&
                  (field === "random" ? (
                    <IconShuffle size={16} />
                  ) : (
                    <IconChevronDown
                      size={14}
                      className={`sort-dir-chevron${albumSort.direction === "asc" ? " asc" : ""}`}
                    />
                  ))}
              </button>
            );
          })}
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Close
        </button>
      </div>
    </div>,
    document.body,
  );
}
