import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import PlaylistPickerContent from "../components/PlaylistPickerContent";
import { pushBackHandler } from "../lib/backHandler";

interface Props {
  /** Resolved lazily so album flows can fetch their track list on demand. */
  getTrackIds: () => Promise<string[]>;
  /** Sheet heading, e.g. the track or album title being added. */
  heading: string;
  /** Skip the picker and go straight to naming a new playlist
   * ("Save Queue as Playlist…"). */
  createOnly?: boolean;
  /** Layer above the now-playing sheet (z-1100) when opened from it. */
  overSheet?: boolean;
  onDismiss: () => void;
}

/**
 * Mobile action-sheet shell around PlaylistPickerContent (the desktop
 * counterpart is PlaylistPickerModal). The naming step tags the backdrop
 * `text-entry` so the sheet rides above the software keyboard.
 */
export default function PlaylistPickerSheet({
  getTrackIds,
  heading,
  createOnly,
  overSheet,
  onDismiss,
}: Props) {
  const [naming, setNaming] = useState(!!createOnly);

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
      className={`mobile-action-sheet-backdrop${overSheet ? " over-sheet" : ""}${naming ? " text-entry" : ""}`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="mobile-action-sheet">
        <div className="mobile-action-sheet-group">
          <div className="mobile-action-sheet-header">
            {naming ? "Name the new playlist" : `Add “${heading}” to a playlist`}
          </div>
          <PlaylistPickerContent
            getTrackIds={getTrackIds}
            createOnly={createOnly}
            onDone={onDismiss}
            onNamingChange={setNaming}
          />
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Cancel
        </button>
      </div>
    </div>,
    document.body,
  );
}
