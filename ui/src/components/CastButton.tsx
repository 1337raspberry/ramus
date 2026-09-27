import { useCallback, useState } from "react";
import { useCastStore } from "../stores/castStore";
import { useConnectionStore } from "../stores/connectionStore";
import { IconCast } from "./Icons";
import CastPickerModal from "./CastPickerModal";
import CastPickerSheet from "../mobile/CastPickerSheet";

/**
 * Opens the player picker. Accented while playback is on another device.
 * Offline it can still open while casting, so playback can be handed back.
 */
export default function CastButton({
  className,
  size,
  sheet,
}: {
  className: string;
  size?: number;
  /** Open the mobile action sheet instead of the desktop modal. */
  sheet?: boolean;
}) {
  const casting = useCastStore((s) => s.player !== null);
  const offline = useConnectionStore((s) => s.effectiveOffline);
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);

  return (
    <>
      <button
        type="button"
        className={`${className}${casting ? " active" : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(true);
        }}
        disabled={offline && !casting}
        aria-label="Play on…"
        title="Play on…"
      >
        <IconCast size={size} />
      </button>
      {open &&
        (sheet ? (
          <CastPickerSheet overSheet onDismiss={close} />
        ) : (
          <CastPickerModal onDismiss={close} />
        ))}
    </>
  );
}
