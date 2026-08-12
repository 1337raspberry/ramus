import { useCallback, useEffect, useRef } from "react";

interface Options {
  /** Fired when a row's removal commits (full swipe, fast flick, or tapping
   * the revealed button). Receives the row index. */
  onDelete: (index: number) => void;
  /** Label on the revealed action button. */
  label?: string;
}

/** Settled reveal width — fits the action label. */
const REVEAL_WIDTH = 76;
/** Horizontal px of intent before the gesture is claimed from the scroller. */
const CLAIM_SLOP = 10;
/** Fraction of row width past which release commits the removal. */
const COMMIT_FRACTION = 0.5;
/** Leftward px/ms at release that commits regardless of distance (the
 * "hard swipe"), once past the reveal width. */
const COMMIT_VELOCITY = 0.6;
/** Opaque surface while the row slides — rows are transparent over the app
 * backdrop, so the action layer would bleed through the content. */
const SLIDE_BG = "rgba(28, 28, 32, 0.96)";

/**
 * Swipe-left-to-remove for fixed-height list rows (native-style): a soft
 * swipe settles open revealing an action button, a hard/full swipe commits
 * immediately. Touch-only — desktop keeps its explicit buttons.
 *
 * Transforms are written imperatively during the gesture (same rule as
 * `useListReorder` — a setState per touchmove re-renders the whole list),
 * and moves attach NON-PASSIVE document listeners at touchstart because
 * React pointer events die on mobile scrollers. Vertical intent inside the
 * slop abandons the gesture so scrolling stays native; `touch-action: pan-y`
 * on the content element keeps Android from claiming horizontal pans first.
 *
 * The red action layer is NOT part of the React markup — the hook creates
 * one imperatively, only for a row that's actually displaced. These lists
 * are unvirtualized and can run 1000+ rows: a per-row action layer in JSX
 * adds two DOM nodes per row and inflates every list reconcile for an
 * element almost nobody ever sees. (Same reason the content must NOT carry
 * `will-change: transform` — that promotes every row to its own compositor
 * layer, which wrecks the now-playing sheet's open/close drag at queue
 * scale. The gesture's own transform promotes the one moving row for free.)
 *
 * Markup contract per row (the container needs `.swipe-row`, which clips):
 *
 *   <div className="… swipe-row">
 *     <div className="… swipe-row-content"
 *          ref={(el) => swipe.setContentRef(i, el)}
 *          {...swipe.contentProps(i)}>
 *       …row content…
 *     </div>
 *   </div>
 */
export function useSwipeToDelete({ onDelete, label = "Remove" }: Options) {
  const contentsRef = useRef<(HTMLElement | null)[]>([]);
  const openRef = useRef<number | null>(null);
  const suppressClickRef = useRef(false);
  const onDeleteRef = useRef(onDelete);
  onDeleteRef.current = onDelete;
  const labelRef = useRef(label);
  labelRef.current = label;

  /** Find or lazily create the action layer inside this row's container.
   * `data-index` is restamped on every displacement, so the button always
   * removes the row it's currently attached to even after indices shift. */
  const actionOf = (el: HTMLElement, create: boolean): HTMLElement | null => {
    const parent = el.parentElement;
    if (!parent) return null;
    let action = parent.querySelector<HTMLElement>(":scope > .swipe-row-action");
    if (!action && create) {
      action = document.createElement("div");
      action.className = "swipe-row-action";
      const btn = document.createElement("button");
      btn.textContent = labelRef.current;
      btn.tabIndex = -1;
      btn.addEventListener("click", () => {
        const idx = Number(action!.dataset.index);
        if (!Number.isNaN(idx)) deleteNowRef.current(idx);
      });
      action.appendChild(btn);
      parent.insertBefore(action, el);
    }
    return action;
  };

  const setOffset = (index: number, px: number, animate: boolean) => {
    const el = contentsRef.current[index];
    if (!el) return;
    el.style.transition = animate ? "transform 0.18s ease" : "none";
    el.style.transform = px ? `translateX(${px}px)` : "";
    el.style.background = px ? SLIDE_BG : "";
    // Rows are transparent over the app backdrop, so the action layer must
    // stay hidden except while its own row is displaced.
    const action = actionOf(el, px !== 0);
    if (!action) return;
    if (px) {
      action.dataset.index = String(index);
      action.style.visibility = "visible";
    } else if (!animate) {
      action.style.visibility = "hidden";
    } else {
      // Keep the red visible while the row animates shut; a fresh gesture
      // in the window leaves a transform behind, which skips the hide.
      window.setTimeout(() => {
        if (!el.style.transform) action.style.visibility = "hidden";
      }, 200);
    }
  };

  const closeOpenRow = useCallback((animate = true) => {
    const open = openRef.current;
    openRef.current = null;
    if (open !== null) setOffset(open, 0, animate);
  }, []);

  useEffect(() => () => closeOpenRow(false), [closeOpenRow]);

  const setContentRef = useCallback((index: number, el: HTMLElement | null) => {
    contentsRef.current[index] = el;
    // Virtualized consumers unmount rows that scroll far away; an open row
    // that unmounts loses its inline transform, so the open state must not
    // survive it (a later touch would start from a phantom offset).
    if (el === null && openRef.current === index) openRef.current = null;
  }, []);

  /** Slide the row out and commit the removal. */
  const deleteNow = useCallback((index: number) => {
    const el = contentsRef.current[index];
    openRef.current = null;
    if (el) {
      const action = actionOf(el, true);
      if (action) {
        action.dataset.index = String(index);
        action.style.visibility = "visible";
      }
      el.style.transition = "transform 0.15s ease";
      el.style.transform = `translateX(${-(el.offsetWidth || 300)}px)`;
    }
    // Let the slide-out land before React drops the row.
    window.setTimeout(() => {
      onDeleteRef.current(index);
      // Indices shift on re-render; whatever element now sits at this slot
      // must not inherit the slid-out transform.
      const el2 = contentsRef.current[index];
      if (el2) {
        el2.style.transition = "none";
        el2.style.transform = "";
        el2.style.background = "";
        const action2 = actionOf(el2, false);
        if (action2) action2.style.visibility = "hidden";
      }
    }, 150);
  }, []);
  const deleteNowRef = useRef(deleteNow);
  deleteNowRef.current = deleteNow;

  const armClickSuppression = () => {
    suppressClickRef.current = true;
    window.setTimeout(() => {
      suppressClickRef.current = false;
    }, 350);
  };

  const contentProps = (index: number) => ({
    onTouchStart: (e: React.TouchEvent) => {
      // Reorder handles own their gesture.
      if ((e.target as Element).closest(".playlist-grab, .mobile-upnext-grab")) return;
      const wasOpen = openRef.current;
      if (wasOpen !== null && wasOpen !== index) {
        // A touch anywhere else first closes the open row (native behaviour)
        // and the accompanying tap must not activate the row underneath.
        closeOpenRow();
        armClickSuppression();
        return;
      }
      const t = e.touches[0];
      const startX = t.clientX;
      const startY = t.clientY;
      const base = wasOpen === index ? -REVEAL_WIDTH : 0;
      const el = contentsRef.current[index];
      const width = el?.offsetWidth ?? 300;
      let claimed = false;
      let offset = base;
      let lastX = startX;
      let lastT = performance.now();
      let velocity = 0;

      const detach = () => {
        document.removeEventListener("touchmove", move);
        document.removeEventListener("touchend", up);
        document.removeEventListener("touchcancel", cancel);
      };

      const move = (ev: TouchEvent) => {
        const touch = ev.touches[0];
        if (!touch) return;
        const dx = touch.clientX - startX;
        const dy = touch.clientY - startY;
        if (!claimed) {
          if (Math.abs(dy) > Math.abs(dx) && Math.abs(dy) > CLAIM_SLOP) {
            // Vertical intent — it's a scroll; stand down for good.
            detach();
            return;
          }
          const horizontal = Math.abs(dx) > CLAIM_SLOP && Math.abs(dx) > Math.abs(dy);
          if (!horizontal) return;
          if (dx < 0 || base < 0) {
            claimed = true;
          } else {
            // Rightward from closed — nothing to reveal on that side.
            detach();
            return;
          }
        }
        ev.preventDefault();
        const now = performance.now();
        if (now > lastT) velocity = (touch.clientX - lastX) / (now - lastT);
        lastX = touch.clientX;
        lastT = now;
        offset = Math.min(0, Math.max(-width, base + dx));
        setOffset(index, offset, false);
      };

      const finish = (commitAllowed: boolean) => {
        detach();
        if (!claimed) return;
        armClickSuppression();
        const hardSwipe = velocity < -COMMIT_VELOCITY && offset < -REVEAL_WIDTH;
        if (commitAllowed && (offset < -width * COMMIT_FRACTION || hardSwipe)) {
          deleteNow(index);
        } else if (offset < -REVEAL_WIDTH / 2) {
          openRef.current = index;
          setOffset(index, -REVEAL_WIDTH, true);
        } else {
          openRef.current = null;
          setOffset(index, 0, true);
        }
      };

      const up = () => finish(true);
      const cancel = () => finish(false);
      document.addEventListener("touchmove", move, { passive: false });
      document.addEventListener("touchend", up);
      document.addEventListener("touchcancel", cancel);
    },
    onClickCapture: (e: React.MouseEvent) => {
      // Swallow the tap that follows a claimed swipe (or an open-row close).
      if (suppressClickRef.current) {
        e.preventDefault();
        e.stopPropagation();
        suppressClickRef.current = false;
      }
    },
  });

  return { setContentRef, contentProps, deleteNow, closeOpenRow };
}
