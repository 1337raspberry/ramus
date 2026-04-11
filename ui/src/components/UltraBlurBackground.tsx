import type { CSSProperties } from "react";
import type { UltraBlurColors } from "../lib/types";
import { hexToRgb } from "../lib/vibrantColor";

// Curated color presets sampled from real album art.
// Each entry has ultrablur corner colors and a matching accent.
const PRESETS: { accent: [number, number, number]; blur: UltraBlurColors }[] = [
  {
    accent: [26, 221, 255],
    blur: { topLeft: "5a0319", topRight: "890523", bottomRight: "94082c", bottomLeft: "b0012e" },
  },
  {
    accent: [106, 174, 117],
    blur: { topLeft: "08380c", topRight: "037203", bottomRight: "954026", bottomLeft: "681b04" },
  },
  {
    accent: [51, 137, 230],
    blur: { topLeft: "1e2957", topRight: "97394c", bottomRight: "54652f", bottomLeft: "2b6866" },
  },
  {
    accent: [30, 250, 193],
    blur: { topLeft: "043729", topRight: "a41f58", bottomRight: "a22555", bottomLeft: "823439" },
  },
  {
    accent: [241, 207, 69],
    blur: { topLeft: "312f0f", topRight: "695e1e", bottomRight: "665f26", bottomLeft: "695e20" },
  },
  {
    accent: [249, 32, 57],
    blur: { topLeft: "022c5e", topRight: "b10519", bottomRight: "0656ac", bottomLeft: "0d427f" },
  },
  {
    accent: [252, 243, 29],
    blur: { topLeft: "511715", topRight: "382e6b", bottomRight: "832e26", bottomLeft: "a52a1d" },
  },
  {
    accent: [254, 26, 35],
    blur: { topLeft: "1c0303", topRight: "440403", bottomRight: "260303", bottomLeft: "420403" },
  },
  {
    accent: [251, 233, 30],
    blur: { topLeft: "281674", topRight: "47215e", bottomRight: "5b38cb", bottomLeft: "4a1c60" },
  },
  {
    accent: [204, 77, 119],
    blur: { topLeft: "35101c", topRight: "1d4372", bottomRight: "a91847", bottomLeft: "46131f" },
  },
  {
    accent: [247, 103, 34],
    blur: { topLeft: "0b2f52", topRight: "082340", bottomRight: "155488", bottomLeft: "183963" },
  },
  {
    accent: [80, 137, 203],
    blur: { topLeft: "480d53", topRight: "761d62", bottomRight: "4d2354", bottomLeft: "761858" },
  },
  {
    accent: [98, 182, 161],
    blur: { topLeft: "042b32", topRight: "072a35", bottomRight: "072531", bottomLeft: "031d23" },
  },
  {
    accent: [240, 85, 54],
    blur: { topLeft: "10312e", topRight: "146152", bottomRight: "0b2f2a", bottomLeft: "156053" },
  },
  {
    accent: [242, 235, 53],
    blur: { topLeft: "222a34", topRight: "252658", bottomRight: "324885", bottomLeft: "98393c" },
  },
  {
    accent: [255, 26, 26],
    blur: { topLeft: "1f040c", topRight: "310511", bottomRight: "280311", bottomLeft: "4d0304" },
  },
  {
    accent: [201, 239, 82],
    blur: { topLeft: "123713", topRight: "40582a", bottomRight: "183f16", bottomLeft: "296e3a" },
  },
  {
    accent: [238, 216, 51],
    blur: { topLeft: "370b2b", topRight: "9b2671", bottomRight: "992875", bottomLeft: "41193e" },
  },
  {
    accent: [254, 27, 205],
    blur: { topLeft: "270423", topRight: "520347", bottomRight: "400436", bottomLeft: "850473" },
  },
  {
    accent: [232, 181, 74],
    blur: { topLeft: "153617", topRight: "055f54", bottomRight: "130f31", bottomLeft: "6c2f03" },
  },
  {
    accent: [254, 229, 27],
    blur: { topLeft: "43174e", topRight: "a01c44", bottomRight: "ad1a12", bottomLeft: "a21b42" },
  },
];

/** Pick a random preset and apply its accent + return its blur colors. */
export function randomPalette(): { colors: UltraBlurColors; accent: [number, number, number] } {
  const preset = PRESETS[Math.floor(Math.random() * PRESETS.length)];
  return { colors: preset.blur, accent: preset.accent };
}

// --- Color helpers ---

/**
 * Baked tone adjustments from the tuning session. Brightness and
 * saturation are applied in JS (not CSS `filter:`) because CSS filters
 * go through Chromium's filter pipeline — the same pipeline whose
 * 8-bit intermediate surfaces banded on Windows when we were using
 * `filter: blur()`. Doing the tone math in JS guarantees the values
 * feeding the gradient are already "final", so the only path the
 * colours take to the screen is the high-precision gradient path.
 *
 * Saturation is a per-channel blend toward the RGB mean — simple
 * grey in sRGB, not perceptually uniform, but deterministic and
 * cheap. Brightness is a scalar multiply clamped to [0, 255].
 * Applied in saturation-then-brightness order.
 */
const BRIGHTNESS = 0.9;
const SATURATION = 1.2;

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
 * Full-window gradient background built from 4 overlapping CSS
 * `radial-gradient()` layers, one anchored at each corner.
 *
 * This replaces an earlier approach that used 4 solid-colour circle
 * divs inside a `filter: blur(220px)` wrapper. That approach looked
 * fine on macOS but banded catastrophically on Windows because
 * Chromium's CSS filter pipeline uses 8-bit intermediate surfaces,
 * baking quantisation into the blur output before any downstream
 * dither could help. CSS radial gradients go through a different
 * render path with higher internal precision and built-in dithering,
 * so the same visual works cleanly on both platforms.
 *
 * The 4 corner colours are passed as CSS custom properties on the
 * element's inline style. The matching `@property` declarations in
 * `styles.css` with `syntax: '<color>'` enable smooth colour-space
 * interpolation on the `transition`, giving us a 0.8s soft crossfade
 * whenever the album changes. Brightness + saturation (baked as
 * constants above) are applied to each colour in JS before it
 * becomes CSS — deliberately NOT via CSS `filter:`, which would
 * put us back in the banding-prone filter pipeline.
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
