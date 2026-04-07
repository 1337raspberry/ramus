import type { UltraBlurColors } from "../lib/types";

// 20 preset earth-tone hex colors from the Swift reference implementation.
const PALETTE = [
  "7a3b3b", "8b5e5e", "7a5a3b", "8b7a5e", "6b6b3b",
  "3b6b4a", "4a7a5e", "3b6b6b", "3b5a6b", "3b4a7a",
  "5e5e8b", "6b3b6b", "7a5a7a", "5e3b4a", "4a5e6b",
  "6b5a4a", "5a6b5a", "4a5a5e", "7a6b5a", "5e4a5a",
];

function pick(): string {
  return PALETTE[Math.floor(Math.random() * PALETTE.length)];
}

export function randomPalette(): UltraBlurColors {
  return {
    topLeft: pick(),
    topRight: pick(),
    bottomRight: pick(),
    bottomLeft: pick(),
  };
}

// --- Color math (ported from Swift UltraBlurBackground.swift) ---

interface RGB { r: number; g: number; b: number }

function hexToRgb(hex: string): RGB {
  const cleaned = hex.startsWith("#") ? hex.slice(1) : hex;
  if (cleaned.length !== 6) return { r: 0, g: 0, b: 0 };
  const value = parseInt(cleaned, 16);
  if (isNaN(value)) return { r: 0, g: 0, b: 0 };
  return {
    r: ((value >> 16) & 0xff) / 255,
    g: ((value >> 8) & 0xff) / 255,
    b: (value & 0xff) / 255,
  };
}

function hue2rgb(p: number, q: number, t: number): number {
  if (t < 0) t += 1;
  if (t > 1) t -= 1;
  if (t < 1 / 6) return p + (q - p) * 6 * t;
  if (t < 1 / 2) return q;
  if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
  return p;
}

/** Desaturate 35%, darken 25%, enforce minimum lightness 0.20. */
function pastelise(c: RGB): RGB {
  const maxC = Math.max(c.r, c.g, c.b);
  const minC = Math.min(c.r, c.g, c.b);
  let l = (maxC + minC) / 2;
  const delta = maxC - minC;

  if (delta <= 0.001) {
    const darkened = Math.max(l * 0.75, 0.12);
    return { r: darkened, g: darkened, b: darkened };
  }

  let s = l < 0.5 ? delta / (maxC + minC) : delta / (2 - maxC - minC);
  s *= 0.65;
  l = Math.max(l * 0.75, 0.20);

  let h: number;
  if (maxC === c.r) {
    h = (c.g - c.b) / delta + (c.g < c.b ? 6 : 0);
  } else if (maxC === c.g) {
    h = (c.b - c.r) / delta + 2;
  } else {
    h = (c.r - c.g) / delta + 4;
  }
  h /= 6;

  const q2 = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p2 = 2 * l - q2;
  return {
    r: hue2rgb(p2, q2, h + 1 / 3),
    g: hue2rgb(p2, q2, h),
    b: hue2rgb(p2, q2, h - 1 / 3),
  };
}

function toCSS(c: RGB): string {
  return `rgb(${Math.round(c.r * 255)}, ${Math.round(c.g * 255)}, ${Math.round(c.b * 255)})`;
}

interface Props {
  colors: UltraBlurColors;
}

/**
 * Full-window gradient background using 4 blurred color blobs.
 * CSS blur eliminates all 8-bit banding, and background-color
 * is natively transitionable for smooth crossfades.
 */
export default function UltraBlurBackground({ colors }: Props) {
  const tl = toCSS(pastelise(hexToRgb(colors.topLeft)));
  const tr = toCSS(pastelise(hexToRgb(colors.topRight)));
  const bl = toCSS(pastelise(hexToRgb(colors.bottomLeft)));
  const br = toCSS(pastelise(hexToRgb(colors.bottomRight)));

  return (
    <div className="ultrablur-bg">
      <div className="ultrablur-blob ultrablur-tl" style={{ backgroundColor: tl }} />
      <div className="ultrablur-blob ultrablur-tr" style={{ backgroundColor: tr }} />
      <div className="ultrablur-blob ultrablur-bl" style={{ backgroundColor: bl }} />
      <div className="ultrablur-blob ultrablur-br" style={{ backgroundColor: br }} />
    </div>
  );
}
