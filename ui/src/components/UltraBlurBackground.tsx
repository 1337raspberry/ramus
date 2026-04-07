import type { UltraBlurColors } from "../lib/types";

// Curated color presets sampled from real album art.
// Each entry has ultrablur corner colors and a matching accent.
const PRESETS: { accent: [number, number, number]; blur: UltraBlurColors }[] = [
  { accent: [26, 221, 255], blur: { topLeft: "5a0319", topRight: "890523", bottomRight: "94082c", bottomLeft: "b0012e" } },
  { accent: [106, 174, 117], blur: { topLeft: "08380c", topRight: "037203", bottomRight: "954026", bottomLeft: "681b04" } },
  { accent: [51, 137, 230], blur: { topLeft: "1e2957", topRight: "97394c", bottomRight: "54652f", bottomLeft: "2b6866" } },
  { accent: [30, 250, 193], blur: { topLeft: "043729", topRight: "a41f58", bottomRight: "a22555", bottomLeft: "823439" } },
  { accent: [241, 207, 69], blur: { topLeft: "312f0f", topRight: "695e1e", bottomRight: "665f26", bottomLeft: "695e20" } },
  { accent: [249, 32, 57], blur: { topLeft: "022c5e", topRight: "b10519", bottomRight: "0656ac", bottomLeft: "0d427f" } },
  { accent: [252, 243, 29], blur: { topLeft: "511715", topRight: "382e6b", bottomRight: "832e26", bottomLeft: "a52a1d" } },
  { accent: [254, 26, 35], blur: { topLeft: "1c0303", topRight: "440403", bottomRight: "260303", bottomLeft: "420403" } },
  { accent: [251, 233, 30], blur: { topLeft: "281674", topRight: "47215e", bottomRight: "5b38cb", bottomLeft: "4a1c60" } },
  { accent: [204, 77, 119], blur: { topLeft: "35101c", topRight: "1d4372", bottomRight: "a91847", bottomLeft: "46131f" } },
  { accent: [247, 103, 34], blur: { topLeft: "0b2f52", topRight: "082340", bottomRight: "155488", bottomLeft: "183963" } },
  { accent: [80, 137, 203], blur: { topLeft: "480d53", topRight: "761d62", bottomRight: "4d2354", bottomLeft: "761858" } },
  { accent: [98, 182, 161], blur: { topLeft: "042b32", topRight: "072a35", bottomRight: "072531", bottomLeft: "031d23" } },
  { accent: [240, 85, 54], blur: { topLeft: "10312e", topRight: "146152", bottomRight: "0b2f2a", bottomLeft: "156053" } },
  { accent: [242, 235, 53], blur: { topLeft: "222a34", topRight: "252658", bottomRight: "324885", bottomLeft: "98393c" } },
  { accent: [255, 26, 26], blur: { topLeft: "1f040c", topRight: "310511", bottomRight: "280311", bottomLeft: "4d0304" } },
  { accent: [201, 239, 82], blur: { topLeft: "123713", topRight: "40582a", bottomRight: "183f16", bottomLeft: "296e3a" } },
  { accent: [238, 216, 51], blur: { topLeft: "370b2b", topRight: "9b2671", bottomRight: "992875", bottomLeft: "41193e" } },
  { accent: [254, 27, 205], blur: { topLeft: "270423", topRight: "520347", bottomRight: "400436", bottomLeft: "850473" } },
  { accent: [232, 181, 74], blur: { topLeft: "153617", topRight: "055f54", bottomRight: "130f31", bottomLeft: "6c2f03" } },
  { accent: [254, 229, 27], blur: { topLeft: "43174e", topRight: "a01c44", bottomRight: "ad1a12", bottomLeft: "a21b42" } },
];

/** Pick a random preset and apply its accent + return its blur colors. */
export function randomPalette(): { colors: UltraBlurColors; accent: [number, number, number] } {
  const preset = PRESETS[Math.floor(Math.random() * PRESETS.length)];
  return { colors: preset.blur, accent: preset.accent };
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
