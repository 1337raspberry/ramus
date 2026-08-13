import { useCallback, useEffect, useRef } from "react";

interface Options {
  count: number;
  rowHeight: number;
  /** Fired once, at drop, with the source and destination indices. */
  onReorder: (from: number, to: number) => void;
}

/**
 * Drag-to-reorder for a fixed-row-height list, driven from a per-row grab
 * handle. Transforms are written imperatively during the gesture — a setState
 * per move would re-render the whole list at gesture rate (same rule as the
 * now-playing sheet drag). React state changes once, at drop, via
 * `onReorder`.
 *
 * Mobile gotcha: React pointer events fail on mobile scrollers (iOS's
 * scroller claims the gesture; Android fires pointercancel on vertical pan),
 * so the handle attaches NON-PASSIVE document touch listeners at touchstart
 * and preventDefault()s every move. Desktop rides mousedown/mousemove.
 */
export function useListReorder({ count, rowHeight, onReorder }: Options) {
  const rowsRef = useRef<(HTMLElement | null)[]>([]);
  const dragRef = useRef<{ from: number; startY: number; target: number } | null>(null);
  // Kept fresh so a gesture spanning a re-render sees current values.
  const optsRef = useRef({ count, rowHeight, onReorder });
  optsRef.current = { count, rowHeight, onReorder };

  const setRowRef = useCallback((index: number, el: HTMLElement | null) => {
    rowsRef.current[index] = el;
  }, []);

  const begin = (index: number, clientY: number) => {
    dragRef.current = { from: index, startY: clientY, target: index };
    const row = rowsRef.current[index];
    if (row) {
      row.style.zIndex = "10";
      row.style.position = "relative";
      row.style.transition = "none";
      // Rows are normally transparent over the app backdrop; the dragged one
      // slides over its siblings, so it needs an opaque surface to read.
      row.style.background = "rgba(28, 28, 32, 0.96)";
    }
  };

  const update = (clientY: number) => {
    const d = dragRef.current;
    if (!d) return;
    const { count, rowHeight } = optsRef.current;
    const delta = clientY - d.startY;
    const dragged = rowsRef.current[d.from];
    if (dragged) dragged.style.transform = `translateY(${delta}px)`;
    let target = d.from + Math.round(delta / rowHeight);
    target = Math.max(0, Math.min(count - 1, target));
    if (target === d.target) return;
    d.target = target;
    for (let i = 0; i < count; i++) {
      if (i === d.from) continue;
      const el = rowsRef.current[i];
      if (!el) continue;
      let shift = 0;
      if (d.from < target && i > d.from && i <= target) shift = -rowHeight;
      else if (d.from > target && i >= target && i < d.from) shift = rowHeight;
      el.style.transition = "transform 0.15s";
      el.style.transform = shift ? `translateY(${shift}px)` : "";
    }
  };

  const resetRows = () => {
    for (let i = 0; i < optsRef.current.count; i++) {
      const el = rowsRef.current[i];
      if (!el) continue;
      el.style.transform = "";
      el.style.transition = "";
      el.style.zIndex = "";
      el.style.position = "";
      el.style.background = "";
    }
  };

  const end = (commit: boolean) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d) return;
    resetRows();
    if (commit && d.target !== d.from) optsRef.current.onReorder(d.from, d.target);
  };

  // A drag holds indices into a list whose numbering can move underneath it:
  // when the playing track advances, every upcoming row shifts one slot down
  // and each per-index ref is re-bound to a different element, while the
  // caller's onReorder closure picks up the new offset. The captured
  // from/target pair then names the wrong track, so the move lands on a
  // neighbour with no visible sign it went astray. The gesture cannot be
  // rebased once its index space moves — drop it and let the user re-grab.
  // Nulling the drag is enough to disarm the in-flight listeners: their
  // move/end handlers both no-op on a null drag, and they detach themselves.
  useEffect(() => {
    if (!dragRef.current) return;
    dragRef.current = null;
    resetRows();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [count]);

  const handleProps = (index: number) => ({
    onTouchStart: (e: React.TouchEvent) => {
      begin(index, e.touches[0].clientY);
      const move = (ev: TouchEvent) => {
        ev.preventDefault();
        update(ev.touches[0].clientY);
      };
      const detach = () => {
        document.removeEventListener("touchmove", move);
        document.removeEventListener("touchend", up);
        document.removeEventListener("touchcancel", cancel);
      };
      const up = () => {
        detach();
        end(true);
      };
      const cancel = () => {
        detach();
        end(false);
      };
      document.addEventListener("touchmove", move, { passive: false });
      document.addEventListener("touchend", up);
      document.addEventListener("touchcancel", cancel);
    },
    onMouseDown: (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      begin(index, e.clientY);
      const move = (ev: MouseEvent) => update(ev.clientY);
      const up = () => {
        document.removeEventListener("mousemove", move);
        document.removeEventListener("mouseup", up);
        end(true);
      };
      document.addEventListener("mousemove", move);
      document.addEventListener("mouseup", up);
    },
  });

  return { setRowRef, handleProps };
}
