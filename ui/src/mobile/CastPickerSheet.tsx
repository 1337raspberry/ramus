import { useEffect } from "react";
import { createPortal } from "react-dom";
import CastPickerContent from "../components/CastPickerContent";
import { pushBackHandler } from "../lib/backHandler";

/**
 * Mobile action-sheet shell around CastPickerContent (the desktop
 * counterpart is CastPickerModal). Portals to <body>, as the other action
 * sheets do.
 */
export default function CastPickerSheet({
  overSheet,
  onDismiss,
}: {
  /** Layer above the now-playing sheet (z-1100) when opened from it. */
  overSheet?: boolean;
  onDismiss: () => void;
}) {
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
          <div className="mobile-action-sheet-header">Play on</div>
          <CastPickerContent onDone={onDismiss} />
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Cancel
        </button>
      </div>
    </div>,
    document.body,
  );
}
