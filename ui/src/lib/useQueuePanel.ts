import { useCallback, useRef, useState } from "react";

/**
 * Shared state machine for the "wheel down to reveal the upcoming queue,
 * scroll back to the top to collapse it" pattern used by both DetailColumn
 * (compact Now Playing panel) and FocusNowPlayingView.
 *
 * Returns an `open`/`setOpen` pair plus event handlers that can be spread
 * onto a scrollable container. Uses a ref alongside state so the wheel
 * callback doesn't need to be re-bound (and close over stale props) on
 * every re-render.
 */
export function useQueuePanel() {
  const [open, setOpenState] = useState(false);
  const openRef = useRef(false);

  const setOpen = useCallback((next: boolean) => {
    openRef.current = next;
    setOpenState(next);
  }, []);

  const toggle = useCallback(() => {
    setOpen(!openRef.current);
  }, [setOpen]);

  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      if (e.deltaY > 0 && !openRef.current) {
        setOpen(true);
      }
    },
    [setOpen],
  );

  const onScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      if (openRef.current && e.currentTarget.scrollTop === 0) {
        setOpen(false);
      }
    },
    [setOpen],
  );

  return { open, setOpen, toggle, onWheel, onScroll };
}
