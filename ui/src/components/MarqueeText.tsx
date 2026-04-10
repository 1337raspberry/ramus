import {
  useLayoutEffect,
  useRef,
  type CSSProperties,
  type MouseEventHandler,
  type ReactNode,
} from "react";

interface Props {
  children: ReactNode;
  /**
   * Applied to the outer wrapper so existing classes like
   * `.focus-track-title`, `.focus-album-title`, `.np-track-title` etc.
   * continue to supply font sizing, flex layout, and clipping.
   */
  className?: string;
  onClick?: MouseEventHandler<HTMLDivElement>;
  title?: string;
  /** Scroll speed while the animation is in motion (px per second). */
  speedPxPerSec?: number;
  /** How long to hold at each end before reversing direction. */
  holdMs?: number;
  /**
   * Optional inline style for the outer wrapper. Useful for callers that
   * need to pass things like `color` without creating a dedicated class.
   */
  style?: CSSProperties;
}

/**
 * Text that scrolls back-and-forth (marquee) when it overflows its
 * container, and sits still otherwise. Measures the rendered content
 * against the wrapper using `scrollWidth` vs `clientWidth`, toggles a
 * `marquee--active` class to start a CSS keyframe animation, and
 * publishes the scroll distance + total cycle duration as CSS custom
 * properties (`--marquee-dist`, `--marquee-duration`).
 *
 * A `ResizeObserver` re-measures when the wrapper's width changes, so
 * resizing the window activates / deactivates the marquee on the fly.
 * `children` is also in the effect deps, so swapping tracks resets the
 * animation to the start.
 *
 * Keep the outer element a `div` so callers can drop it into existing
 * flex rows that expect a block-level child; inner text sits inside a
 * `span.marquee-inner` which is the element that actually translates.
 */
export default function MarqueeText({
  children,
  className,
  onClick,
  title,
  speedPxPerSec = 40,
  holdMs = 1500,
  style,
}: Props) {
  const outerRef = useRef<HTMLDivElement | null>(null);
  const innerRef = useRef<HTMLSpanElement | null>(null);

  useLayoutEffect(() => {
    const outer = outerRef.current;
    const inner = innerRef.current;
    if (!outer || !inner) return;

    const measure = () => {
      // `scrollWidth` reports the full text width even when the span has
      // `overflow: hidden`, so we can measure without toggling styles.
      const overflow = inner.scrollWidth - outer.clientWidth;
      if (overflow > 1) {
        // Time to scroll one direction, at the configured speed.
        const oneWayMs = (overflow / speedPxPerSec) * 1000;
        // Full cycle: scroll right + hold + scroll back + hold.
        const durationMs = oneWayMs * 2 + holdMs * 2;
        // Drop the active class first, force a synchronous reflow, then
        // re-add. Without the reflow the browser keeps the previous
        // animation timeline running, so a new (long) track would pick
        // up the marquee mid-cycle instead of starting from the left.
        outer.classList.remove("marquee--active");
        void inner.offsetWidth;
        inner.style.setProperty("--marquee-dist", `${-overflow}px`);
        inner.style.setProperty("--marquee-duration", `${durationMs}ms`);
        outer.classList.add("marquee--active");
      } else {
        outer.classList.remove("marquee--active");
        inner.style.removeProperty("--marquee-dist");
        inner.style.removeProperty("--marquee-duration");
      }
    };

    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(outer);
    return () => ro.disconnect();
  }, [children, speedPxPerSec, holdMs]);

  return (
    <div
      ref={outerRef}
      className={className ? `marquee ${className}` : "marquee"}
      onClick={onClick}
      title={title}
      style={style}
    >
      <span ref={innerRef} className="marquee-inner">
        {children}
      </span>
    </div>
  );
}
