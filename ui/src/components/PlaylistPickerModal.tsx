import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import PlaylistPickerContent from "./PlaylistPickerContent";

interface Props {
  /** Resolved lazily so album flows can fetch their track list on demand. */
  getTrackIds: () => Promise<string[]>;
  /** Dialog subject, e.g. the track or album title being added. */
  heading: string;
  /** Skip the picker and go straight to naming a new playlist
   * ("Save Queue as Playlist…"). */
  createOnly?: boolean;
  onDismiss: () => void;
}

/**
 * Desktop settings-chassis shell around PlaylistPickerContent (the mobile
 * counterpart is PlaylistPickerSheet). Clears the focus overlay the same
 * way as CollectionPickerModal.
 */
export default function PlaylistPickerModal({
  getTrackIds,
  heading,
  createOnly,
  onDismiss,
}: Props) {
  const [naming, setNaming] = useState(!!createOnly);

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
        aria-labelledby="playlist-picker-title"
      >
        <div className="settings-header">
          <h2 id="playlist-picker-title">{naming ? "New Playlist" : "Add to Playlist"}</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            x
          </button>
        </div>
        <div className="settings-body">
          <div className="picker-modal-hint">
            {naming ? `Name the new playlist for “${heading}”` : `Add “${heading}” to a playlist`}
          </div>
          <PlaylistPickerContent
            getTrackIds={getTrackIds}
            createOnly={createOnly}
            onDone={onDismiss}
            onNamingChange={setNaming}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}
