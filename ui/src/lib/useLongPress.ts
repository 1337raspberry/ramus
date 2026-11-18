import { useCallback, useEffect, useRef } from "react";

import { beginHoldRumble, type HoldRumble } from "./haptics";

interface Options {
  /** Omit to disable the hold entirely — the element keeps plain tap
   * semantics and no timer is armed, so a long hold can't swallow the
   * subsequent click. */
  onLongPress?: () => void;
  onClick?: () => void;
  /** Hold duration before the long-press fires. */
  ms?: number;
  /** Squared pixel distance past which a touch is treated as a scroll and the
   * pending long-press is cancelled. */
  moveCancelSq?: number;
  /** Observes the pending hold's lifecycle: `true` when a hold timer arms,
   * `false` when it fires or is cancelled. Lets callers render hold progress
   * (e.g. a fill ring) for long `ms` values. */
  onHoldChange?: (holding: boolean) => void;
  /** How far into the hold its haptic rumble begins. The default keeps a
   * touch that becomes a scroll silent; an element that cannot scroll can
   * start sooner, so the rumble warns of the hold while there is still time
   * to let go. */
  rumbleDelayMs?: number;
}

/**
 * Shared touch long-press behaviour: a hold fires `onLongPress`, a plain tap
 * fires `onClick`, and the click that always follows a long-press is swallowed.
 * Movement past the threshold (i.e. a scroll) cancels the pending hold. Spread
 * the returned handlers onto any element:
 *
 *   const lp = useLongPress({ onLongPress, onClick });
 *   <button {...lp}>…</button>
 */
export function useLongPress({
  onLongPress,
  onClick,
  ms = 500,
  moveCancelSq = 100,
  onHoldChange,
  rumbleDelayMs,
}: Options) {
  const timerRef = useRef<number | null>(null);
  const longPressedRef = useRef(false);
  const startRef = useRef<{ x: number; y: number } | null>(null);
  const rumbleRef = useRef<HoldRumble | null>(null);

  const clear = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
      // A pending hold given up (a lift, a scroll, an unmount): cut its
      // rumble short.
      rumbleRef.current?.cancel();
      rumbleRef.current = null;
      onHoldChange?.(false);
    }
  }, [onHoldChange]);

  useEffect(() => () => clear(), [clear]);

  const onTouchStart = useCallback(
    (e: React.TouchEvent) => {
      longPressedRef.current = false;
      const t = e.touches[0];
      startRef.current = t ? { x: t.clientX, y: t.clientY } : null;
      clear();
      if (!onLongPress) return;
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null;
        longPressedRef.current = true;
        rumbleRef.current?.fire();
        rumbleRef.current = null;
        onHoldChange?.(false);
        onLongPress();
      }, ms);
      // A rumble builds under the finger until the hold fires.
      rumbleRef.current = beginHoldRumble(ms, rumbleDelayMs);
      onHoldChange?.(true);
    },
    [clear, ms, onLongPress, onHoldChange, rumbleDelayMs],
  );

  const onTouchMove = useCallback(
    (e: React.TouchEvent) => {
      const start = startRef.current;
      const t = e.touches[0];
      if (!start || !t) return;
      const dx = t.clientX - start.x;
      const dy = t.clientY - start.y;
      if (dx * dx + dy * dy > moveCancelSq) clear();
    },
    [clear, moveCancelSq],
  );

  const handleClick = useCallback(() => {
    if (longPressedRef.current) {
      // Swallow the click that fires after a long-press.
      longPressedRef.current = false;
      return;
    }
    onClick?.();
  }, [onClick]);

  const onContextMenu = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      if (!onLongPress) return;
      // Some platforms synthesize contextmenu from a touch long-press that
      // also fired the timer; the guard avoids a double-fire.
      if (longPressedRef.current) return;
      // Disarm a hold still pending, or its timer would fire the long press
      // a second time.
      clear();
      longPressedRef.current = true;
      onLongPress();
    },
    [clear, onLongPress],
  );

  return {
    onClick: handleClick,
    onTouchStart,
    onTouchMove,
    onTouchEnd: clear,
    onTouchCancel: clear,
    onContextMenu,
  };
}
