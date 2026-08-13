import { useEffect } from "react";
import { createPortal } from "react-dom";
import type { Album } from "../lib/types";
import CollectionPickerContent from "../components/CollectionPickerContent";
import { pushBackHandler } from "../lib/backHandler";

interface Props {
  album: Album;
  /** Layer above the now-playing sheet (z-1100) when opened from it. */
  overSheet?: boolean;
  onDismiss: () => void;
}

/**
 * Mobile action-sheet shell around CollectionPickerContent (the desktop
 * counterpart is CollectionPickerModal). Portals to <body> like the other
 * mobile action sheets — callers inside virtualizer rows or the now-playing
 * sheet can't host a fixed overlay themselves (transformed ancestors become
 * its containing block).
 */
export default function CollectionPickerSheet({ album, overSheet, onDismiss }: Props) {
  // Hardware back closes the picker, not whatever sits underneath it.
  useEffect(
    () =>
      pushBackHandler(() => {
        onDismiss();
        return true;
      }),
    [onDismiss],
  );

  return createPortal(
    <div
      className={`mobile-action-sheet-backdrop${overSheet ? " over-sheet" : ""}`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="mobile-action-sheet">
        <div className="mobile-action-sheet-group">
          <div className="mobile-action-sheet-header">
            Collections for “{album.title}” — tap to add or remove
          </div>
          <CollectionPickerContent album={album} />
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Done
        </button>
      </div>
    </div>,
    document.body,
  );
}
