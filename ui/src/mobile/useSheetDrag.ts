import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { createArtMorph } from "./artMorph";

/** Movement before a touch is treated as a drag rather than a tap. */
const CLAIM_SLOP = 3;
/** Fraction of the travel that commits the gesture on distance alone. */
const COMMIT_FRACTION = 0.12;
/** …or this much speed, in px/ms, in the committing direction. */
const COMMIT_VELOCITY = 0.4;
/** Trailing window the release velocity is measured over. */
const VELOCITY_WINDOW_MS = 100;
/** Rate the sheet keeps following at once dragged past its resting bounds. */
const RUBBER_BAND = 0.35;
/** Settle duration, scaled between these by the distance still to travel. */
const SETTLE_MIN_MS = 180;
const SETTLE_MAX_MS = 380;
/** Duration of the class-driven open/close in styles.css — the mini-player and
 *  the art morph have to match it when something other than a drag moves the
 *  sheet. */
const CLASS_TRANSITION_MS = 300;
const CURVE = "cubic-bezier(0.2, 0.9, 0.2, 1)";
/** Custom property the sheet's content rows read their opacity from. */
const CONTENT_OPACITY = "--np-content-opacity";
/** Custom property that collapses the mini-player's backdrop to its own box. */
const MINI_BG_EXTEND = "--mini-bg-extend";
/** Published so the sheet's closed transform rests on the mini-player's frame
 *  rather than off the bottom of the screen. */
const MINI_HEIGHT = "--np-mini-height";

function clamp01(v: number): number {
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

/**
 * Sheet content opacity for a given open fraction. Front-loaded rather than
 * linear: the full player is solid for the top two thirds of the travel and
 * only dissolves toward the mini-player, which is the direction that has to
 * read as a crossfade between two states rather than a panel sliding away.
 */
function contentOpacity(p: number): number {
  return clamp01((p - 0.02) / 0.28);
}

/** Mini-player opacity, handed off to the sheet content just as that arrives —
 *  otherwise both are legible at once and the track title reads twice. */
function miniOpacity(p: number): number {
  return clamp01(1 - p / 0.32);
}

export interface SheetDragRefs {
  sheet: RefObject<HTMLDivElement | null>;
  /** Scroll container — a downward drag on the hero art only becomes a
   *  dismiss once this is at the top. */
  body: RefObject<HTMLDivElement | null>;
  header: RefObject<HTMLElement | null>;
  heroArt: RefObject<HTMLDivElement | null>;
  mini: RefObject<HTMLDivElement | null>;
  miniArt: RefObject<HTMLElement | null>;
}

interface Options {
  expanded: boolean;
  /** Resolved art URL, or null for a placeholder track (morph is skipped). */
  artSrc: string | null;
  onExpand: () => void;
  onCollapse: () => void;
}

function velocity(samples: { y: number; t: number }[]): number {
  if (samples.length < 2) return 0;
  const first = samples[0];
  const last = samples[samples.length - 1];
  const dt = last.t - first.t;
  return dt > 0 ? (last.y - first.y) / dt : 0;
}

function currentTranslateY(el: HTMLElement): number {
  const t = getComputedStyle(el).transform;
  if (!t || t === "none") return 0;
  try {
    return new DOMMatrixReadOnly(t).m42;
  } catch {
    return 0;
  }
}

/**
 * Drag behaviour for the mobile now-playing sheet: pull up from the
 * mini-player to open, pull down from the header or the hero art to dismiss.
 *
 * The sheet's closed position is the mini-player's own frame, not the bottom
 * of the screen, and the mini-player rides with its TOP edge pinned to the
 * sheet's. Both matter for the same reason: whatever the finger grabbed stays
 * under the finger for the whole gesture. Anchoring the panel a bar-height
 * lower — closed off-screen, or the bar's bottom edge on the sheet's top edge
 * — leaves the panel trailing the finger by that gap, which becomes obvious
 * the moment the bar fades out and there is visibly nothing under the touch.
 *
 * The sheet's transform is written imperatively rather than through React
 * state. A `setState` per `touchmove` re-renders the whole sheet — the
 * waveform, both gradient backdrops and the unvirtualized Up Next list — on
 * every frame of the gesture, which is enough to miss frames on a long queue.
 * Nothing here touches React until the gesture is released.
 *
 * The CSS transition is suppressed for the duration of the drag, which is the
 * other half of the same problem: left on, every `touchmove` restarts a 300ms
 * eased animation toward the new finger position, so the sheet coasts whenever
 * two moves report the same coordinate and lurches when the next one lands.
 * That reads as jitter at slow drag speeds and is invisible at flick speed.
 *
 * Everything the sheet element needs during a gesture goes through inline
 * styles rather than a class, because React owns its `className` and rewrites
 * the attribute wholesale whenever `expanded` flips — which happens in the
 * middle of the settle.
 *
 * Committing takes distance OR velocity, so a short flick opens the sheet and
 * a slow drag past ~12% of the travel does too, while a flick back the other
 * way cancels whatever the distance says.
 */
export function useSheetDrag(refs: SheetDragRefs, opts: Options): void {
  const { expanded } = opts;

  // Listeners are registered once and read everything current from here, so a
  // track change or a new callback identity never re-binds them mid-gesture.
  const live = useRef(opts);
  live.current = opts;

  const morphRef = useRef<ReturnType<typeof createArtMorph> | null>(null);
  if (!morphRef.current) morphRef.current = createArtMorph();

  const drag = useRef({
    claimed: false,
    /** 1 = dragging down (toward dismissed), -1 = dragging up (toward open). */
    dir: 1 as 1 | -1,
    claimY: 0,
    base: 0,
    translate: 0,
    /** Distance between the open and closed positions — NOT the sheet height. */
    travel: 0,
    samples: [] as { y: number; t: number }[],
  });
  const settle = useRef<{ active: boolean; timer: number; off: (() => void) | null }>({
    active: false,
    timer: 0,
    off: null,
  });
  /** Set when a settle ends collapsed: the inline transform has to outlive the
   *  settle until React has actually dropped `.expanded`. */
  const resetPending = useRef(false);
  const mounted = useRef(false);
  /** Cleanup for a mini-player animation driven by the non-drag path, which has
   *  no transition of its own to hang a settle on. */
  const miniTimer = useRef(0);
  const animateMiniRef = useRef<((toExpanded: boolean) => void) | null>(null);

  useEffect(() => {
    const morph = morphRef.current!;

    const clearSettleWatch = () => {
      if (settle.current.timer) window.clearTimeout(settle.current.timer);
      settle.current.off?.();
      settle.current = { active: false, timer: 0, off: null };
    };

    /** Travel between the open and closed positions, and the closed translate —
     *  they are the same number, since open is translate 0. */
    const measureTravel = (): number => {
      const sheet = refs.sheet.current;
      const h = sheet?.offsetHeight || window.innerHeight;
      const miniH = refs.mini.current?.offsetHeight ?? 0;
      // Published for the closed transform in styles.css, so the class-driven
      // open/close animates between the same two positions a drag does.
      sheet?.style.setProperty(MINI_HEIGHT, `${miniH}px`);
      return Math.max(1, h - miniH);
    };

    /** `true`/`false` pin it; `null` hands it back to the class, which shows
     *  the sheet only while `.expanded` is on. */
    const setSheetVisible = (v: boolean | null) => {
      const sheet = refs.sheet.current;
      if (!sheet) return;
      sheet.style.visibility = v === null ? "" : v ? "visible" : "hidden";
    };

    const releaseSheet = () => {
      const sheet = refs.sheet.current;
      if (!sheet) return;
      sheet.style.transition = "";
      sheet.style.transform = "";
      sheet.style.visibility = "";
      // Back to the class value, which already matches whichever state the
      // sheet just settled into.
      sheet.style.removeProperty(CONTENT_OPACITY);
    };

    /**
     * Lift the mini-player over the sheet. Closed, the sheet rests exactly on
     * the bar's frame, so the bar has to paint on top of it or it is simply
     * gone the instant a drag starts.
     */
    const setMiniLifted = (on: boolean) => {
      const mini = refs.mini.current;
      if (mini) mini.style.zIndex = on ? "101" : "";
    };

    const applyMini = (translate: number, travel: number) => {
      const mini = refs.mini.current;
      if (!mini) return;
      // Top edge pinned to the sheet's top edge: at the closed position the two
      // frames coincide, so this is zero and the bar sits where it always does.
      mini.style.transform = `translate3d(0, ${translate - travel}px, 0)`;
      mini.style.opacity = String(miniOpacity(clamp01(1 - translate / travel)));
      // Its backdrop layers run 100vh past the bar so the home-indicator area
      // stays filled. With the bar painting above the sheet that slab would
      // cover the whole panel, so pull it back to the box.
      mini.style.setProperty(MINI_BG_EXTEND, "0px");
    };

    const releaseMini = () => {
      const mini = refs.mini.current;
      if (!mini) return;
      if (miniTimer.current) {
        window.clearTimeout(miniTimer.current);
        miniTimer.current = 0;
      }
      mini.style.transition = "";
      mini.style.transform = "";
      mini.style.opacity = "";
      mini.style.removeProperty(MINI_BG_EXTEND);
    };

    const applyFrame = (translate: number, travel: number) => {
      const sheet = refs.sheet.current;
      if (!sheet) return;
      sheet.style.transform = `translate3d(0, ${translate}px, 0)`;
      const p = clamp01(1 - translate / travel);
      morph.setProgress(p);
      sheet.style.setProperty(CONTENT_OPACITY, String(contentOpacity(p)));
      applyMini(translate, travel);
    };

    const settleTo = (target: 0 | 1, from: number, travel: number) => {
      const sheet = refs.sheet.current;
      if (!sheet) return;
      const targetPx = target === 0 ? 0 : travel;
      const remaining = Math.abs(targetPx - from) / travel;
      const ms = Math.round(SETTLE_MIN_MS + remaining * (SETTLE_MAX_MS - SETTLE_MIN_MS));

      clearSettleWatch();
      settle.current.active = true;

      // Opening flips React now, so `.expanded` is in place by the time the
      // resting transform is handed back to it; the inline value wins in the
      // meantime, so there is nothing to see. Closing has to wait for the
      // transition to finish or the sheet jumps home for a frame.
      if (target === 0) live.current.onExpand();

      sheet.style.transition = `transform ${ms}ms ${CURVE}, ${CONTENT_OPACITY} ${ms}ms ${CURVE}`;
      const mini = refs.mini.current;
      if (mini) mini.style.transition = `transform ${ms}ms ${CURVE}, opacity ${ms}ms ${CURVE}`;
      void sheet.offsetHeight;
      sheet.style.transform = `translate3d(0, ${targetPx}px, 0)`;
      sheet.style.setProperty(CONTENT_OPACITY, target === 0 ? "1" : "0");
      applyMini(targetPx, travel);
      morph.settle(target === 0 ? 1 : 0, ms);

      const finish = () => {
        if (!settle.current.active) return;
        clearSettleWatch();
        releaseMini();
        if (target === 0) {
          // Fully open: the sheet covers the bar, so it can drop back
          // underneath immediately — and must, or it paints over the player.
          setMiniLifted(false);
          releaseSheet();
        } else if (live.current.expanded) {
          // Closed. Hiding the sheet and dropping the bar back underneath it
          // have to land in the SAME style recalc — the sheet comes to rest
          // exactly on the bar's frame, so any gap between the two shows a
          // blank panel where the mini-player should be. Both happen here,
          // synchronously; only the transform is handed back later, from the
          // layout effect, once React has actually removed `.expanded`.
          setSheetVisible(false);
          setMiniLifted(false);
          resetPending.current = true;
          live.current.onCollapse();
        } else {
          setMiniLifted(false);
          releaseSheet();
        }
      };
      // transitionend bubbles up from every animated descendant, so the sheet
      // itself has to be the target.
      const onEnd = (e: TransitionEvent) => {
        if (e.target === sheet && e.propertyName === "transform") finish();
      };
      sheet.addEventListener("transitionend", onEnd);
      settle.current.off = () => sheet.removeEventListener("transitionend", onEnd);
      settle.current.timer = window.setTimeout(finish, ms + 120);
    };

    const beginDrag = (dir: 1 | -1, y: number) => {
      const sheet = refs.sheet.current;
      if (!sheet) return;
      const d = drag.current;
      const interrupted = settle.current.active;
      clearSettleWatch();

      d.claimed = true;
      d.dir = dir;
      d.claimY = y;
      d.travel = measureTravel();
      // Picking a drag up from wherever an interrupted settle had got to keeps
      // a grab-mid-animation from snapping.
      d.base = interrupted ? currentTranslateY(sheet) : dir === 1 ? 0 : d.travel;
      d.translate = d.base;
      d.samples = [{ y, t: performance.now() }];

      // The sheet is still `visibility: hidden` when it is being pulled up out
      // of the mini-player, before `.expanded` has been applied.
      sheet.style.visibility = "visible";
      sheet.style.transition = "none";
      if (miniTimer.current) {
        window.clearTimeout(miniTimer.current);
        miniTimer.current = 0;
      }
      setMiniLifted(true);
      if (refs.mini.current) refs.mini.current.style.transition = "none";

      // A morph left over from an interrupted settle is re-measured rather
      // than reused: the sheet has moved since it was set up.
      morph.begin(live.current.artSrc, refs.miniArt.current, refs.heroArt.current, sheet);
      applyFrame(d.translate, d.travel);
    };

    const moveDrag = (y: number) => {
      const d = drag.current;
      if (!d.claimed) return;
      const raw = d.base + (y - d.claimY);
      // Past either end the sheet still follows, at a third of the rate — the
      // pushback that says this is the end of the travel.
      d.translate =
        raw < 0
          ? raw * RUBBER_BAND
          : raw > d.travel
            ? d.travel + (raw - d.travel) * RUBBER_BAND
            : raw;

      const now = performance.now();
      d.samples.push({ y, t: now });
      while (d.samples.length > 2 && now - d.samples[0].t > VELOCITY_WINDOW_MS) d.samples.shift();

      applyFrame(d.translate, d.travel);
    };

    const endDrag = () => {
      const d = drag.current;
      if (!d.claimed) return;
      d.claimed = false;

      const v = velocity(d.samples);
      const travelled = d.dir === 1 ? d.translate : d.travel - d.translate;
      const flick = d.dir === 1 ? v > COMMIT_VELOCITY : v < -COMMIT_VELOCITY;
      const flickBack = d.dir === 1 ? v < -COMMIT_VELOCITY : v > COMMIT_VELOCITY;
      const commit = !flickBack && (flick || travelled > d.travel * COMMIT_FRACTION);

      // Dragging down commits to dismissed; dragging up commits to open.
      settleTo((d.dir === 1) === commit ? 1 : 0, d.translate, d.travel);
    };

    // --- Gesture sources -------------------------------------------------
    // All three use non-passive `touchmove` rather than pointer events:
    // Android's Chromium WebView fires `pointercancel` the moment it decides a
    // vertical drag belongs to a scroller, so a pointer-based version fails
    // there while looking fine on iOS WebKit.

    const cleanups: Array<() => void> = [];

    interface Source {
      /** Evaluated when the slop is crossed, NOT at touchstart: a drag on the
       *  hero art may begin as a scroll-back and only become a dismiss once
       *  the body has bottomed out at the top. `target` is the touchstart
       *  element (touchmove reports the same one for the whole gesture). */
      canClaim: (target: HTMLElement | null) => boolean;
      /**
       * Consulted for moves BEFORE the slop is crossed. Returning true calls
       * `preventDefault()` without starting the drag, which takes the gesture
       * off a native scroller that would otherwise latch onto it during those
       * first few pixels — see the note on `attach`.
       */
      holdBeforeClaim?: (dy: number) => boolean;
      onTouchStart?: () => void;
      onClaim?: () => void;
    }

    /**
     * `preventDefault()` on `touchmove` only keeps a native scroller off the
     * gesture if it lands before that scroller starts scrolling; once WebKit
     * has handed the gesture to its scrolling thread, later calls are ignored
     * for the rest of the touch. Sources sitting inside a scroll container
     * therefore have to stake their claim within the slop window, via
     * `holdBeforeClaim` — otherwise a drag that reverses direction mid-gesture
     * ends up driving the sheet AND scrolling the container underneath it.
     */
    const attach = (el: HTMLElement, dir: 1 | -1, source: Source) => {
      let startY: number | null = null;
      let skip = false;
      /** Whether THIS source owns the active drag — a second finger landing on
       *  another source must not settle a gesture it did not start. */
      let owns = false;

      const onStart = (e: TouchEvent) => {
        source.onTouchStart?.();
        skip = e.touches.length !== 1 || drag.current.claimed;
        startY = skip ? null : e.touches[0].clientY;
      };

      const onMove = (e: TouchEvent) => {
        if (startY == null || skip) return;
        const y = e.touches[0].clientY;
        if (!owns) {
          const dy = y - startY;
          if (dir === 1 ? dy <= CLAIM_SLOP : dy >= -CLAIM_SLOP) {
            if (source.holdBeforeClaim?.(dy)) e.preventDefault();
            return;
          }
          if (drag.current.claimed) return;
          // Not a hard skip: the guard may pass on a later move.
          if (!source.canClaim(e.target as HTMLElement | null)) return;
          owns = true;
          source.onClaim?.();
          beginDrag(dir, y);
          return;
        }
        // Only legal because the listener is registered `{ passive: false }`.
        e.preventDefault();
        moveDrag(y);
      };

      const onEnd = () => {
        if (owns) endDrag();
        owns = false;
        startY = null;
        skip = false;
      };

      el.addEventListener("touchstart", onStart, { passive: true });
      el.addEventListener("touchmove", onMove, { passive: false });
      el.addEventListener("touchend", onEnd, { passive: true });
      el.addEventListener("touchcancel", onEnd, { passive: true });
      cleanups.push(() => {
        el.removeEventListener("touchstart", onStart);
        el.removeEventListener("touchmove", onMove);
        el.removeEventListener("touchend", onEnd);
        el.removeEventListener("touchcancel", onEnd);
      });
    };

    const mini = refs.mini.current;
    if (mini) {
      // A drag that starts on the art button would otherwise still fire its
      // click on release, expanding the sheet even when the drag was cancelled.
      let swallowClick = false;
      const onClickCapture = (e: MouseEvent) => {
        if (!swallowClick) return;
        swallowClick = false;
        e.stopPropagation();
        e.preventDefault();
      };
      mini.addEventListener("click", onClickCapture, true);
      cleanups.push(() => mini.removeEventListener("click", onClickCapture, true));

      attach(mini, -1, {
        onTouchStart: () => {
          swallowClick = false;
        },
        canClaim: (target) => {
          if (live.current.expanded) return false;
          // The transport row and the scrubber own their own touches. The art
          // button does not, so a drag can start there and a tap still opens.
          return !target?.closest(".mobile-miniplayer-wave, .mobile-miniplayer-controls");
        },
        onClaim: () => {
          swallowClick = true;
        },
      });
    }

    const header = refs.header.current;
    if (header) {
      attach(header, 1, {
        canClaim: (target) => !target?.closest('button, [role="button"]'),
      });
    }

    const heroArt = refs.heroArt.current;
    if (heroArt) {
      // Lyrics mode owns its own scrolling and tap-to-seek inside this box.
      const dismissable = () =>
        !usePlaybackStore.getState().showLyrics && (refs.body.current?.scrollTop ?? 0) <= 0;
      attach(heroArt, 1, {
        canClaim: dismissable,
        // Unlike the header, this box lives INSIDE `.mobile-sheet-body`, so the
        // gesture is up for grabs. Downward movement at the top of the body
        // cannot scroll anything, which makes it free to claim immediately —
        // and claiming it here is what stops the scroller taking the gesture
        // during the slop window and then scrolling the queue when the drag
        // reverses back up. Upward movement is left alone, so a swipe from the
        // artwork still scrolls to Up Next.
        holdBeforeClaim: (dy) => dy > 0 && dismissable(),
      });
    }

    // The non-drag path (tap, hardware back) has to fly the mini-player
    // alongside the sheet's class transition — nothing else moves it there.
    animateMiniRef.current = (toExpanded: boolean) => {
      const mini = refs.mini.current;
      if (!mini) return;
      const travel = measureTravel();
      setMiniLifted(true);
      // Closing, the class rule would drop `visibility` the instant `.expanded`
      // came off, cutting the slide short — pin it for the length of it.
      setSheetVisible(toExpanded ? null : true);
      mini.style.transition = "none";
      applyMini(toExpanded ? travel : 0, travel);
      void mini.offsetWidth;
      mini.style.transition = `transform ${CLASS_TRANSITION_MS}ms ${CURVE}, opacity ${CLASS_TRANSITION_MS}ms ${CURVE}`;
      applyMini(toExpanded ? 0 : travel, travel);
      miniTimer.current = window.setTimeout(() => {
        releaseMini();
        // Same single-recalc rule as the settle path: unpin the sheet and drop
        // the bar together, never one frame apart.
        if (!toExpanded) setSheetVisible(null);
        setMiniLifted(false);
      }, CLASS_TRANSITION_MS + 40);
    };

    return () => {
      cleanups.forEach((fn) => fn());
      clearSettleWatch();
      releaseMini();
      setMiniLifted(false);
      morph.dispose();
      animateMiniRef.current = null;
      drag.current.claimed = false;
    };
    // Bound once: every changing value is read through `live`/`refs`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Open/close that did not come from a drag — a tap on the mini art, hardware
  // back, a navigation that closes the sheet. The class transition moves the
  // sheet on its own; the mini-player and the art have to be flown alongside.
  useLayoutEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    if (resetPending.current) {
      resetPending.current = false;
      const sheet = refs.sheet.current;
      if (sheet) {
        sheet.style.transition = "";
        sheet.style.transform = "";
        sheet.style.visibility = "";
        sheet.style.removeProperty(CONTENT_OPACITY);
      }
      return;
    }
    if (drag.current.claimed || settle.current.active) return;

    animateMiniRef.current?.(expanded);

    const morph = morphRef.current;
    if (!morph) return;
    const flying = morph.begin(
      opts.artSrc,
      refs.miniArt.current,
      refs.heroArt.current,
      refs.sheet.current,
    );
    if (flying) {
      morph.setProgress(expanded ? 0 : 1);
      morph.settle(expanded ? 1 : 0, CLASS_TRANSITION_MS);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expanded]);
}
