/**
 * The clear screen's timed-lyrics strip (`components/FocusSingalong.tsx`):
 * its parameters and the placement of its lines. The type size and the
 * strip's position live in styles.css (`.focus-singalong`). The values
 * here are the shipped look, read each time the lines are placed; the
 * object is deliberately mutable so development tooling can adjust them
 * live and re-place with `placeSingalongLines`.
 */
export interface SingalongParams {
  /**
   * Seconds ahead of the playhead the lines are read at. A line change
   * rolls the stack over the CSS transition's 0.32 s, so starting it a
   * beat early lands the line in focus as it is sung rather than after.
   */
  leadS: number;
  /** Size of the lines either side of the focus line, relative to it. */
  neighbourScale: number;
  /** Opacity of the lines either side of the focus line. */
  neighbourOpacity: number;
  /** Vertical gap between the drawn (scaled) lines, in px. */
  gapPx: number;
}

/**
 * Wide enough for the strip beside the corner player (440 px wide, 32 px
 * in from the right edge, with the strip's own insets). Narrower windows
 * leave the strip and its toggle out rather than move them.
 */
export const SINGALONG_QUERY = "(min-width: 1001px)";

export const SINGALONG_PARAMS: SingalongParams = {
  leadS: 0.25,
  neighbourScale: 0.7,
  neighbourOpacity: 0.05,
  gapPx: 19,
};

/**
 * Positions each line in `strip` by its `data-slot`: the focus line (0)
 * centred in the strip, the others stacked above and below it at their
 * scaled heights, and the outer slots (±2) invisible. Lines keep their
 * DOM nodes across a line change (keyed by id), so writing the new
 * transforms makes the CSS transition roll the whole stack by one slot.
 * Written imperatively rather than as React styles so that a line
 * arriving by a seek or a new track can be placed first and then faded
 * in where it lands.
 */
export function placeSingalongLines(strip: HTMLElement): void {
  const { neighbourScale, neighbourOpacity, gapPx } = SINGALONG_PARAMS;
  const bySlot = new Map<number, HTMLElement>();
  for (const child of strip.children) {
    const el = child as HTMLElement;
    bySlot.set(Number(el.dataset.slot), el);
  }
  const scaleOf = (slot: number) => (slot === 0 ? 1 : neighbourScale);
  const opacityOf = (slot: number) =>
    slot === 0 ? 1 : Math.abs(slot) === 1 ? neighbourOpacity : 0;
  // offsetHeight is the unscaled layout height; transforms don't touch it.
  const heightOf = (slot: number) => (bySlot.get(slot)?.offsetHeight ?? 0) * scaleOf(slot);

  const tops = new Map<number, number>([[0, (strip.clientHeight - heightOf(0)) / 2]]);
  for (let slot = -1; slot >= -2; slot--) {
    tops.set(slot, tops.get(slot + 1)! - gapPx - heightOf(slot));
  }
  for (let slot = 1; slot <= 2; slot++) {
    tops.set(slot, tops.get(slot - 1)! + heightOf(slot - 1) + gapPx);
  }

  for (const [slot, el] of bySlot) {
    const transform = `translateY(${tops.get(slot)}px) scale(${scaleOf(slot)})`;
    if (!el.dataset.placed) {
      el.dataset.placed = "1";
      el.style.transition = "none";
      el.style.transform = transform;
      el.style.opacity = "0";
      // Flush the start state so the opacity change below transitions.
      void el.offsetHeight;
      el.style.transition = "";
    } else {
      el.style.transform = transform;
    }
    el.style.opacity = String(opacityOf(slot));
  }
}
