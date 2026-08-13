import { useEffect } from "react";
import { createPortal } from "react-dom";
import type { Album } from "../lib/types";
import CollectionPickerContent from "./CollectionPickerContent";

interface Props {
  album: Album;
  onDismiss: () => void;
}

/**
 * Desktop settings-chassis shell around CollectionPickerContent (the mobile
 * counterpart is CollectionPickerSheet). The z-1000 backdrop clears the
 * focus overlay (z-500), so the player's … menu can open it from focus mode
 * too; `useAppKeyboard` yields all shortcuts while a settings backdrop is
 * mounted.
 */
export default function CollectionPickerModal({ album, onDismiss }: Props) {
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

  return createPortal(
    <div
      className="settings-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div
        className="settings-panel glass picker-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="collection-picker-title"
      >
        <div className="settings-header">
          <h2 id="collection-picker-title">Add to Collection</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            x
          </button>
        </div>
        <div className="settings-body">
          <div className="picker-modal-hint">
            Collections for “{album.title}” — click to add or remove
          </div>
          <CollectionPickerContent album={album} />
          <div className="bookmark-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="bookmark-btn" onClick={onDismiss}>
              Done
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
