import type { CSSProperties } from "react";
import type { UltraBlurColors } from "../lib/types";
import { isHDR } from "../lib/hdr";
import { hexToRgb } from "../lib/vibrantColor";

// --- Color helpers ---

/**
 * Baked tone adjustments. Brightness and saturation are applied in JS,
 * NOT via CSS `filter:`: Chromium's filter pipeline uses 8-bit
 * intermediate surfaces that banded on Windows with the prior
 * `filter: blur()` approach. Doing the math here keeps the colours on
 * the high-precision gradient path all the way to the screen.
 *
 * Saturation is a per-channel blend toward the RGB mean (sRGB grey, not
 * perceptually uniform, but deterministic). Brightness is a scalar
 * multiply clamped to [0, 255]. Order: saturation then brightness.
 */
const BRIGHTNESS = isHDR ? 0.9 : 1.0;
/* Tuned together with the extraction-side CHROMA_CAP in blurArt.ts —
 * the boost revives dull server-fallback colours, while the cap stops
 * already-saturated extracted colours from being pushed to neon. */
const SATURATION = isHDR ? 1.05 : 1.3;

function adjustedCSS(hex: string): string {
  const [r, g, b] = hexToRgb(hex);
  const grey = (r + g + b) / 3;
  const clamp = (c: number) =>
    Math.max(0, Math.min(255, Math.round((grey + (c - grey) * SATURATION) * BRIGHTNESS)));
  return `rgb(${clamp(r)}, ${clamp(g)}, ${clamp(b)})`;
}

interface Props {
  colors: UltraBlurColors;
}

/**
 * Full-window gradient background from 4 overlapping CSS
 * `radial-gradient()` layers, one per corner. Corner colours come from
 * art-derived extraction (`lib/blurArt.ts`) with the server-provided
 * UltraBlur colours as instant first paint before the art decodes.
 *
 * Do NOT use `filter: blur/brightness/saturate` here. A prior
 * implementation used 4 solid-colour divs inside `filter: blur(220px)`
 * and banded on Windows because Chromium's CSS filter pipeline uses
 * 8-bit intermediate surfaces. WKWebView on macOS uses
 * higher-precision Metal-backed intermediates plus window-server
 * dithering, which hid the issue — do not be fooled by Mac-only
 * testing. CSS radial gradients go through a different render path
 * with higher internal precision and built-in dithering. (Rendering a
 * low-res copy of the art itself was also tried: it reads as a
 * pixelated stretched image at any usable resolution.)
 *
 * Corner colours are passed as CSS custom properties. The matching
 * `@property` declarations in `styles.css` with `syntax: '<color>'`
 * enable smooth colour-space interpolation for the 0.8s album
 * crossfade; without them the custom properties transition as string
 * swaps (instant).
 *
 * Brightness + saturation are applied in JS (see `adjustedCSS`)
 * specifically to avoid the CSS filter pipeline.
 */
export default function UltraBlurBackground({ colors }: Props) {
  const bgStyle = {
    "--ultrablur-tl": adjustedCSS(colors.topLeft),
    "--ultrablur-tr": adjustedCSS(colors.topRight),
    "--ultrablur-bl": adjustedCSS(colors.bottomLeft),
    "--ultrablur-br": adjustedCSS(colors.bottomRight),
  } as CSSProperties;

  return <div className="ultrablur-bg" aria-hidden="true" style={bgStyle} />;
}
