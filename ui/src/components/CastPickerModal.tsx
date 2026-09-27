import { useEffect } from "react";
import { createPortal } from "react-dom";
import CastPickerContent from "./CastPickerContent";

/**
 * Desktop settings-chassis shell around CastPickerContent (the mobile
 * counterpart is CastPickerSheet). `useAppKeyboard` yields all shortcuts
 * while a settings backdrop is mounted.
 */
export default function CastPickerModal({ onDismiss }: { onDismiss: () => void }) {
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
        aria-labelledby="cast-picker-title"
      >
        <div className="settings-header">
          <h2 id="cast-picker-title">Play On</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            ×
          </button>
        </div>
        <div className="settings-body">
          <CastPickerContent onDone={onDismiss} />
        </div>
      </div>
    </div>,
    document.body,
  );
}
