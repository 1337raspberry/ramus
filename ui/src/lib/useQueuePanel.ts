import { useCallback, useRef, useState } from "react";

/**
 * State machine for the "wheel down to reveal the upcoming queue, scroll
 * back to top to collapse" pattern used by DetailColumn and
 * FocusNowPlayingView.
 *
 * Returns `{ open, setOpen, toggle, onWheel, onScroll }`. Uses a ref
 * alongside state so the wheel callback can be stable without closing over
 * stale props.
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
