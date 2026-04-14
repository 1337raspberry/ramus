import {
  useLayoutEffect,
  useRef,
  type CSSProperties,
  type MouseEventHandler,
  type ReactNode,
} from "react";

interface Props {
  children: ReactNode;
  /** Applied to the outer wrapper so callers can keep their font/layout classes. */
  className?: string;
  onClick?: MouseEventHandler<HTMLDivElement>;
  title?: string;
  /** Scroll speed during motion (px per second). */
  speedPxPerSec?: number;
  /** Hold duration at each end before reversing. */
  holdMs?: number;
  /** Inline style for the outer wrapper. */
  style?: CSSProperties;
}

/**
 * Text that scrolls back-and-forth when it overflows its container and
 * sits still otherwise. Measures `scrollWidth` vs `clientWidth`, toggles
 * a `marquee--active` class, and publishes `--marquee-dist` and
 * `--marquee-duration` custom properties for the CSS keyframes.
 *
 * A `ResizeObserver` re-measures on width change. `children` is in the
 * effect deps so swapping tracks resets the animation.
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
      // scrollWidth reports full text width even with overflow:hidden,
      // so measurement needs no style toggling.
      const overflow = inner.scrollWidth - outer.clientWidth;
      if (overflow > 1) {
        const oneWayMs = (overflow / speedPxPerSec) * 1000;
        // Full cycle: scroll out + hold + scroll back + hold.
        const durationMs = oneWayMs * 2 + holdMs * 2;
        // Drop the class, force synchronous reflow, re-add. Without
        // the reflow the browser keeps the previous animation timeline
        // running, so a new long track would pick up mid-cycle instead
        // of starting from the left.
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
