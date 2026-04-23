import type { CSSProperties } from "react";
import type { UltraBlurColors } from "../lib/types";
import { isHDR } from "../lib/hdr";
import { hexToRgb } from "../lib/vibrantColor";

// Curated presets sampled from real album art: corner colours + matching accent.
const PRESETS: { accent: [number, number, number]; blur: UltraBlurColors }[] = [
  {
    accent: [199, 81, 147],
    blur: { topLeft: "5c366a", topRight: "310817", bottomRight: "5c366a", bottomLeft: "a7568a" },
  },
  {
    accent: [34, 246, 202],
    blur: { topLeft: "365747", topRight: "041c17", bottomRight: "365747", bottomLeft: "699c9c" },
  },
  {
    accent: [255, 155, 25],
    blur: { topLeft: "4c3424", topRight: "870d04", bottomRight: "4c3424", bottomLeft: "8a584b" },
  },
  {
    accent: [73, 208, 189],
    blur: { topLeft: "573e3e", topRight: "14767e", bottomRight: "573e3e", bottomLeft: "4e9c8a" },
  },
  {
    accent: [255, 7, 26],
    blur: { topLeft: "39556f", topRight: "c70606", bottomRight: "39556f", bottomLeft: "637a9a" },
  },
  {
    accent: [255, 210, 10],
    blur: { topLeft: "625036", topRight: "845c04", bottomRight: "625036", bottomLeft: "977401" },
  },
  {
    accent: [255, 189, 13],
    blur: { topLeft: "603538", topRight: "ba8b14", bottomRight: "603538", bottomLeft: "74a494" },
  },
  {
    accent: [216, 103, 64],
    blur: { topLeft: "486237", topRight: "721417", bottomRight: "486237", bottomLeft: "a8754c" },
  },
  {
    accent: [33, 139, 247],
    blur: { topLeft: "124b86", topRight: "104174", bottomRight: "124b86", bottomLeft: "8ba5c0" },
  },
  {
    accent: [210, 224, 56],
    blur: { topLeft: "3e5443", topRight: "0f1004", bottomRight: "3e5443", bottomLeft: "678e88" },
  },
  {
    accent: [20, 255, 255],
    blur: { topLeft: "2c4c51", topRight: "08577d", bottomRight: "2c4c51", bottomLeft: "6666ad" },
  },
  {
    accent: [138, 203, 78],
    blur: { topLeft: "445c34", topRight: "4c612c", bottomRight: "445c34", bottomLeft: "948464" },
  },
  {
    accent: [19, 220, 255],
    blur: { topLeft: "132625", topRight: "1c789c", bottomRight: "132625", bottomLeft: "a56e5a" },
  },
  {
    accent: [234, 47, 106],
    blur: { topLeft: "343454", topRight: "642451", bottomRight: "343454", bottomLeft: "753e74" },
  },
];

/** Return a random preset's blur colours and matching accent. */
export function randomPalette(): { colors: UltraBlurColors; accent: [number, number, number] } {
  const preset = PRESETS[Math.floor(Math.random() * PRESETS.length)];
  return { colors: preset.blur, accent: preset.accent };
}

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
const SATURATION = isHDR ? 1.2 : 1.5;

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
 * `radial-gradient()` layers, one per corner.
 *
 * Do NOT use `filter: blur/brightness/saturate` here. A prior
 * implementation used 4 solid-colour divs inside `filter: blur(220px)`
 * and banded on Windows because Chromium's CSS filter pipeline uses
 * 8-bit intermediate surfaces. WKWebView on macOS uses
 * higher-precision Metal-backed intermediates plus window-server
 * dithering, which hid the issue — do not be fooled by Mac-only
 * testing. CSS radial gradients go through a different render path
 * with higher internal precision and built-in dithering.
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
