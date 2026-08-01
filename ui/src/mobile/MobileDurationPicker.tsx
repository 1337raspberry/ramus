import { useEffect, useLayoutEffect, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { pushBackHandler } from "../lib/backHandler";
import { useLibraryStore } from "../stores/libraryStore";

// Target-length options: 0h 15m up to 2h 00m in 5-minute steps.
const MIN_MINUTES = 15;
const MAX_MINUTES = 120;
const STEP_MINUTES = 5;

function formatTarget(minutes: number): string {
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

interface Props {
  /** The stopwatch button the menu drops down from. */
  anchorRef: RefObject<HTMLElement | null>;
  onDismiss: () => void;
}

/// A lightweight dropdown of suggestion target lengths. Both the dismiss scrim
/// and the menu are portalled to `body` and positioned against the anchor's
/// rect, so the menu always paints above everything regardless of the header's
/// stacking context.
export default function MobileDurationPicker({ anchorRef, onDismiss }: Props) {
  const target = useLibraryStore((s) => s.suggestionTargetMinutes);
  const setTarget = useLibraryStore((s) => s.setSuggestionTargetMinutes);

  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    const el = anchorRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ top: r.bottom + 4, left: r.left });
  }, [anchorRef]);

  // Android hardware back / edge swipe dismisses the menu, not the whole
  // suggestion screen beneath it.
  useEffect(
    () =>
      pushBackHandler(() => {
        onDismiss();
        return true;
      }),
    [onDismiss],
  );

  const options: number[] = [];
  for (let m = MIN_MINUTES; m <= MAX_MINUTES; m += STEP_MINUTES) options.push(m);

  const choose = (minutes: number | null) => {
    setTarget(minutes);
    onDismiss();
  };

  if (!pos) return null;

  return createPortal(
    <>
      <div className="mobile-duration-scrim" onClick={onDismiss} />
      <div className="mobile-duration-menu" role="listbox" style={{ top: pos.top, left: pos.left }}>
        <button
          type="button"
          className={`mobile-duration-option${target == null ? " active" : ""}`}
          onClick={() => choose(null)}
        >
          Any length
        </button>
        {options.map((m) => (
          <button
            key={m}
            type="button"
            className={`mobile-duration-option${target === m ? " active" : ""}`}
            onClick={() => choose(m)}
          >
            {formatTarget(m)}
          </button>
        ))}
      </div>
    </>,
    document.body,
  );
}
