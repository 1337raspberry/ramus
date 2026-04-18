import { useEffect, useRef, useState } from "react";

const EDGE_WIDTH = 20;
const THRESHOLD = 80;

export function useEdgeSwipeBack(onBack: () => void, enabled = true) {
  const containerRef = useRef<HTMLDivElement>(null);
  const touchIdRef = useRef<number | null>(null);
  const startX = useRef(0);
  const startY = useRef(0);
  const currentDelta = useRef(0);
  const locked = useRef(false);
  const [swipeX, setSwipeX] = useState(0);

  const backRef = useRef(onBack);
  backRef.current = onBack;

  useEffect(() => {
    const el = containerRef.current;
    if (!el || !enabled) return;

    const onStart = (e: TouchEvent) => {
      if (touchIdRef.current != null) return;
      const t = e.touches[0];
      if (t.clientX > EDGE_WIDTH) return;
      touchIdRef.current = t.identifier;
      startX.current = t.clientX;
      startY.current = t.clientY;
      currentDelta.current = 0;
      locked.current = false;
    };

    const onMove = (e: TouchEvent) => {
      if (touchIdRef.current == null) return;
      const t = Array.from(e.touches).find((x) => x.identifier === touchIdRef.current);
      if (!t) return;
      const dx = t.clientX - startX.current;
      const dy = Math.abs(t.clientY - startY.current);

      if (!locked.current) {
        if (dy > 10 && dy > Math.abs(dx)) {
          touchIdRef.current = null;
          setSwipeX(0);
          return;
        }
        if (Math.abs(dx) > 10) locked.current = true;
        else return;
      }

      if (dx > 0) {
        currentDelta.current = dx;
        setSwipeX(dx);
        e.preventDefault();
      }
    };

    const onEnd = () => {
      if (touchIdRef.current == null) return;
      const d = currentDelta.current;
      touchIdRef.current = null;
      currentDelta.current = 0;
      setSwipeX(0);
      if (d >= THRESHOLD) backRef.current();
    };

    const onCancel = () => {
      touchIdRef.current = null;
      currentDelta.current = 0;
      setSwipeX(0);
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd);
    el.addEventListener("touchcancel", onCancel);

    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onCancel);
      setSwipeX(0);
    };
  }, [enabled]);

  return { containerRef, swipeX };
}
