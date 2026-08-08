interface Box {
  top: number;
  left: number;
  size: number;
  radius: number;
}

/** Matches the sheet's own transform easing so the art and the panel it
 *  belongs to arrive together. */
const CURVE = "cubic-bezier(0.2, 0.9, 0.2, 1)";

export interface ArtMorph {
  /**
   * Measure both art boxes and mount the flying clone, hiding the two real
   * ones. Returns false — having changed nothing — when there is no art to
   * fly (placeholder track, refs not mounted yet).
   */
  begin(
    src: string | null,
    miniArt: HTMLElement | null,
    heroArt: HTMLElement | null,
    sheet: HTMLElement | null,
  ): boolean;
  /** Untransitioned per-frame update. `p` 0 = mini-player box, 1 = hero box. */
  setProgress(p: number): void;
  /** Ease to `p` over `ms`, then unmount and restore the real art. */
  settle(p: number, ms: number): void;
  /** Unmount immediately, restoring the real art. */
  dispose(): void;
}

/**
 * Flies the album art between the mini-player thumbnail and the expanded
 * sheet's hero image.
 *
 * The two are separate elements in separate stacking contexts, so neither can
 * travel to the other's box. A third element — a clone appended to <body>
 * above both — makes the trip instead, while the real two are held at
 * `opacity: 0`. Because it is positioned in viewport coordinates it is
 * independent of the sheet's own translate, which is what lets the art track
 * a straight line to its destination while the panel slides.
 *
 * Deliberately raw DOM rather than React: the morph starts on `touchmove`,
 * and a state update there would re-render the whole sheet — including the
 * unvirtualized Up Next list — on the frame the gesture can least afford it.
 */
export function createArtMorph(): ArtMorph {
  let el: HTMLDivElement | null = null;
  let from: Box | null = null;
  let to: Box | null = null;
  let hiddenMini: HTMLElement | null = null;
  let hiddenHero: HTMLElement | null = null;
  let timer = 0;
  let offEnd: (() => void) | null = null;

  const unmount = () => {
    if (timer) {
      window.clearTimeout(timer);
      timer = 0;
    }
    offEnd?.();
    offEnd = null;
    el?.remove();
    el = null;
    from = null;
    to = null;
    if (hiddenMini) hiddenMini.style.opacity = "";
    if (hiddenHero) hiddenHero.style.opacity = "";
    hiddenMini = null;
    hiddenHero = null;
  };

  const write = (p: number) => {
    if (!el || !from || !to) return;
    const size = from.size + (to.size - from.size) * p;
    const scale = size / to.size;
    const x = from.left + (to.left - from.left) * p;
    const y = from.top + (to.top - from.top) * p;
    const radius = from.radius + (to.radius - from.radius) * p;
    el.style.transform = `translate3d(${x}px, ${y}px, 0) scale(${scale})`;
    // Counter the scale so the painted corner stays on the interpolated
    // value instead of shrinking along with the box.
    el.style.borderRadius = `${radius / scale}px`;
  };

  return {
    begin(src, miniArt, heroArt, sheet) {
      unmount();
      if (!src || !miniArt || !heroArt || !sheet) return false;

      const miniRect = miniArt.getBoundingClientRect();
      const heroRect = heroArt.getBoundingClientRect();
      const sheetRect = sheet.getBoundingClientRect();
      if (miniRect.width < 1 || heroRect.width < 1) return false;

      from = {
        top: miniRect.top,
        left: miniRect.left,
        size: miniRect.width,
        radius: parseFloat(getComputedStyle(miniArt).borderTopLeftRadius) || 0,
      };
      // Measured relative to the sheet, so it is the box the art will occupy
      // once the sheet is home whatever translate the sheet sits at right
      // now — parked off-screen, mid-drag, or mid-transition. `.mobile-sheet`
      // is `inset: 0`, so its own rect is exactly its current offset.
      to = {
        top: heroRect.top - sheetRect.top,
        left: heroRect.left - sheetRect.left,
        size: heroRect.width,
        radius: parseFloat(getComputedStyle(heroArt).borderTopLeftRadius) || 0,
      };

      const node = document.createElement("div");
      node.className = "np-art-morph";
      node.setAttribute("aria-hidden", "true");
      // Sized at the destination and scaled DOWN toward the thumbnail, so the
      // image is never resampled above its natural size.
      node.style.width = `${to.size}px`;
      node.style.height = `${to.size}px`;
      const img = document.createElement("img");
      img.src = src;
      img.alt = "";
      node.appendChild(img);
      document.body.appendChild(node);
      el = node;

      hiddenMini = miniArt;
      hiddenHero = heroArt;
      miniArt.style.opacity = "0";
      heroArt.style.opacity = "0";
      return true;
    },

    setProgress(p) {
      if (!el) return;
      el.style.transition = "none";
      write(p);
    },

    settle(p, ms) {
      const node = el;
      if (!node) return;
      node.style.transition = `transform ${ms}ms ${CURVE}, border-radius ${ms}ms ${CURVE}`;
      // Flush the pending transform so the transition has a start value.
      void node.offsetWidth;
      write(p);

      const done = () => {
        if (el === node) unmount();
      };
      const onEnd = (e: TransitionEvent) => {
        if (e.target === node && e.propertyName === "transform") done();
      };
      node.addEventListener("transitionend", onEnd);
      offEnd = () => node.removeEventListener("transitionend", onEnd);
      // A zero-distance settle fires no transitionend at all, so the timer is
      // the real cleanup and the event only makes it prompt.
      timer = window.setTimeout(done, ms + 120);
    },

    dispose: unmount,
  };
}
