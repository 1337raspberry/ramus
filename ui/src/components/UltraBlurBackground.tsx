import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, Dispatch, SetStateAction } from "react";
import type { UltraBlurColors } from "../lib/types";
import { isHDR } from "../lib/hdr";
import { hexToRgb } from "../lib/vibrantColor";
import {
  UltraBlurRenderer,
  fallbackBaseCSS,
  fallbackColourVars,
  fallbackGradientCSS,
  fallbackTransitionCSS,
  type CornerRgb,
  type Rgb,
} from "../lib/ultraBlurField";

// --- Color helpers ---

/**
 * Baked tone adjustments. Saturation is a per-channel blend toward the
 * RGB mean (sRGB grey, not perceptually uniform, but deterministic).
 * Brightness is a scalar multiply clamped to [0, 255]. Order: saturation
 * then brightness.
 */
const BRIGHTNESS = isHDR ? 0.9 : 1.0;
/* Tuned together with the extraction-side CHROMA_CAP in blurArt.ts —
 * the boost revives dull server-fallback colours, while the cap stops
 * already-saturated extracted colours from being pushed to neon. */
const SATURATION = isHDR ? 1.05 : 1.3;

function adjustedRgb(hex: string): Rgb {
  const [r, g, b] = hexToRgb(hex);
  const grey = (r + g + b) / 3;
  const clamp = (c: number) =>
    Math.max(0, Math.min(255, Math.round((grey + (c - grey) * SATURATION) * BRIGHTNESS)));
  return [clamp(r), clamp(g), clamp(b)];
}

/** Overall dim, from `--ultrablur-opacity` (raised on SDR screens). */
function readOpacity(el: Element): number {
  const v = parseFloat(getComputedStyle(el).getPropertyValue("--ultrablur-opacity"));
  return Number.isFinite(v) ? v : 0.8;
}

/** Set once any context creation fails, so later mounts skip straight to CSS. */
let webglUnavailable = false;

interface Props {
  colors: UltraBlurColors;
}

/**
 * Full-window background: four album-art corner colours fading into a
 * near-black base (see `lib/ultraBlurField.ts` for the field and why it
 * is drawn by a dithered WebGL shader rather than CSS gradients). Corner
 * colours come from art-derived extraction (`lib/blurArt.ts`) with the
 * server-provided UltraBlur colours as instant first paint before the art
 * decodes. Colour changes crossfade over `TRANSITION_MS`.
 *
 * Falls back to the CSS-gradient rendering of the same field when WebGL2
 * is unavailable or the context is lost.
 */
export default function UltraBlurBackground({ colors }: Props) {
  const { topLeft, topRight, bottomLeft, bottomRight } = colors;
  const corners = useMemo<CornerRgb>(
    () => ({
      topLeft: adjustedRgb(topLeft),
      topRight: adjustedRgb(topRight),
      bottomLeft: adjustedRgb(bottomLeft),
      bottomRight: adjustedRgb(bottomRight),
    }),
    [topLeft, topRight, bottomLeft, bottomRight],
  );
  const [fallback, setFallback] = useState(webglUnavailable);

  if (fallback) return <GradientFallback corners={corners} />;
  return <ShaderField corners={corners} onFail={setFallback} />;
}

function ShaderField({
  corners,
  onFail,
}: {
  corners: CornerRgb;
  onFail: Dispatch<SetStateAction<boolean>>;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<UltraBlurRenderer | null>(null);
  const initialCorners = useRef(corners);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const renderer = UltraBlurRenderer.create(canvas, () => onFail(true));
    if (!renderer) {
      webglUnavailable = true;
      onFail(true);
      return;
    }
    rendererRef.current = renderer;

    // Size the backing store in exact device pixels: a canvas even one
    // pixel off is resampled by the compositor, which averages the dither
    // away. `device-pixel-content-box` reports the snapped size where the
    // engine supports it; elsewhere, CSS size × devicePixelRatio.
    const measure = () => {
      const r = canvas.getBoundingClientRect();
      renderer.resize(r.width * devicePixelRatio, r.height * devicePixelRatio);
    };
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      const box = entry.devicePixelContentBoxSize?.[0];
      if (box) renderer.resize(box.inlineSize, box.blockSize);
      else measure();
    });
    try {
      observer.observe(canvas, { box: "device-pixel-content-box" });
    } catch {
      observer.observe(canvas);
    }

    // Moving between screens can change the pixel ratio without changing
    // the CSS size, and `--ultrablur-opacity` follows the dynamic range.
    let dprQuery = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
    const onDprChange = () => {
      dprQuery.removeEventListener("change", onDprChange);
      dprQuery = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
      dprQuery.addEventListener("change", onDprChange);
      measure();
    };
    dprQuery.addEventListener("change", onDprChange);
    const rangeQuery = matchMedia("(dynamic-range: high)");
    const onRangeChange = () => renderer.setOpacity(readOpacity(canvas));
    rangeQuery.addEventListener("change", onRangeChange);

    measure();
    renderer.setOpacity(readOpacity(canvas));
    renderer.setColors(initialCorners.current);

    return () => {
      observer.disconnect();
      dprQuery.removeEventListener("change", onDprChange);
      rangeQuery.removeEventListener("change", onRangeChange);
      renderer.dispose();
      rendererRef.current = null;
    };
  }, [onFail]);

  useLayoutEffect(() => {
    rendererRef.current?.setColors(corners);
  }, [corners]);

  return (
    <canvas ref={canvasRef} className="ultrablur-bg" style={CANVAS_STYLE} aria-hidden="true" />
  );
}

/**
 * Inline so the CSS box can never follow the backing store: an unsized
 * canvas is displayed at its backing size, which the resize observer
 * would then grow by devicePixelRatio every pass.
 */
const CANVAS_STYLE: CSSProperties = { display: "block", width: "100%", height: "100%" };

function GradientFallback({ corners }: { corners: CornerRgb }) {
  const style = {
    ...fallbackColourVars(corners),
    backgroundColor: fallbackBaseCSS(),
    backgroundImage: fallbackGradientCSS(),
    transition: fallbackTransitionCSS(),
  } as CSSProperties;
  return <div className="ultrablur-bg ultrablur-fallback" aria-hidden="true" style={style} />;
}
