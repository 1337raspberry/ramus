/**
 * Vibrant color extraction from album art.
 * Ported from Swift VibrantColor.swift — uses simplified MMCQ
 * (Modified Median Cut Quantization) with HSL-based scoring.
 */

interface RGB { r: number; g: number; b: number }
interface HSL { h: number; s: number; l: number }

function rgbToHsl(c: RGB): HSL {
  const maxC = Math.max(c.r, c.g, c.b);
  const minC = Math.min(c.r, c.g, c.b);
  const l = (maxC + minC) / 2;
  const delta = maxC - minC;
  if (delta < 0.001) return { h: 0, s: 0, l };
  const s = l < 0.5 ? delta / (maxC + minC) : delta / (2 - maxC - minC);
  let h: number;
  if (maxC === c.r) h = (c.g - c.b) / delta + (c.g < c.b ? 6 : 0);
  else if (maxC === c.g) h = (c.b - c.r) / delta + 2;
  else h = (c.r - c.g) / delta + 4;
  return { h: h / 6, s, l };
}

function hslToRgb(h: number, s: number, l: number): RGB {
  if (s < 0.001) return { r: l, g: l, b: l };
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const hue2rgb = (t: number) => {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  return { r: hue2rgb(h + 1 / 3), g: hue2rgb(h), b: hue2rgb(h - 1 / 3) };
}

// --- Color box for MMCQ ---

interface ColorBox {
  pixels: RGB[];
  rMin: number; rMax: number;
  gMin: number; gMax: number;
  bMin: number; bMax: number;
}

function makeBox(pixels: RGB[]): ColorBox {
  let rMin = 1, rMax = 0, gMin = 1, gMax = 0, bMin = 1, bMax = 0;
  for (const p of pixels) {
    if (p.r < rMin) rMin = p.r;
    if (p.r > rMax) rMax = p.r;
    if (p.g < gMin) gMin = p.g;
    if (p.g > gMax) gMax = p.g;
    if (p.b < bMin) bMin = p.b;
    if (p.b > bMax) bMax = p.b;
  }
  return { pixels, rMin, rMax, gMin, gMax, bMin, bMax };
}

function longestAxis(box: ColorBox): "r" | "g" | "b" {
  const dr = box.rMax - box.rMin;
  const dg = box.gMax - box.gMin;
  const db = box.bMax - box.bMin;
  if (dr >= dg && dr >= db) return "r";
  if (dg >= dr && dg >= db) return "g";
  return "b";
}

function splitBox(box: ColorBox): [ColorBox, ColorBox] {
  const axis = longestAxis(box);
  const sorted = [...box.pixels].sort((a, b) => a[axis] - b[axis]);
  const mid = Math.floor(sorted.length / 2);
  return [makeBox(sorted.slice(0, mid)), makeBox(sorted.slice(mid))];
}

function averageColor(box: ColorBox): RGB {
  const n = box.pixels.length;
  if (n === 0) return { r: 0, g: 0, b: 0 };
  let rSum = 0, gSum = 0, bSum = 0;
  for (const p of box.pixels) {
    rSum += p.r;
    gSum += p.g;
    bSum += p.b;
  }
  return { r: rSum / n, g: gSum / n, b: bSum / n };
}

function mostVibrant(box: ColorBox): RGB {
  let best = box.pixels[0];
  let bestScore = -1;
  for (const p of box.pixels) {
    const hsl = rgbToHsl(p);
    const lumScore = 1 - Math.abs(hsl.l - 0.5) * 2;
    const score = hsl.s * 0.6 + lumScore * 0.4;
    if (score > bestScore) {
      bestScore = score;
      best = p;
    }
  }
  return best;
}

// --- Main extraction ---

const SAMPLE_SIZE = 100;
const TARGET_BOXES = 32;
const MIN_LIGHTNESS = 0.55;

/**
 * Extract a vibrant accent color from an HTMLImageElement.
 * Returns [r, g, b] in 0-255 range, or null if extraction fails.
 */
export function extractVibrantColor(img: HTMLImageElement): [number, number, number] | null {
  // Draw image to a small canvas
  const canvas = document.createElement("canvas");
  canvas.width = SAMPLE_SIZE;
  canvas.height = SAMPLE_SIZE;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;

  ctx.drawImage(img, 0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
  const imageData = ctx.getImageData(0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
  const data = imageData.data;

  // Extract pixels, filtering out near-black/near-white/transparent
  const pixels: RGB[] = [];
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 128) continue; // skip transparent
    const r = data[i] / 255;
    const g = data[i + 1] / 255;
    const b = data[i + 2] / 255;
    const lum = 0.299 * r + 0.587 * g + 0.114 * b;
    if (lum < 0.05 || lum > 0.95) continue;
    pixels.push({ r, g, b });
  }

  if (pixels.length < 10) return null;

  // MMCQ: split into boxes
  let boxes: ColorBox[] = [makeBox(pixels)];
  const phase1Target = Math.floor(TARGET_BOXES * 0.75);

  // Phase 1: split by population
  while (boxes.length < phase1Target) {
    const largest = boxes.reduce((a, b) => a.pixels.length > b.pixels.length ? a : b);
    if (largest.pixels.length < 2) break;
    boxes = boxes.filter((b) => b !== largest);
    const [a, b] = splitBox(largest);
    boxes.push(a, b);
  }

  // Phase 2: split by population * volume
  while (boxes.length < TARGET_BOXES) {
    const volume = (b: ColorBox) =>
      (b.rMax - b.rMin) * (b.gMax - b.gMin) * (b.bMax - b.bMin);
    const scored = boxes.map((b) => ({ box: b, score: b.pixels.length * volume(b) }));
    scored.sort((a, b) => b.score - a.score);
    const target = scored[0];
    if (target.box.pixels.length < 2) break;
    boxes = boxes.filter((b) => b !== target.box);
    const [a, b] = splitBox(target.box);
    boxes.push(a, b);
  }

  // Score each box's colors
  // Get top 3 dominant colors for distinctiveness scoring
  const dominant = boxes
    .sort((a, b) => b.pixels.length - a.pixels.length)
    .slice(0, 3)
    .map(averageColor)
    .map(rgbToHsl);

  interface Candidate {
    rgb: RGB;
    score: number;
  }

  const candidates: Candidate[] = [];
  for (const box of boxes) {
    const avg = averageColor(box);
    const vib = mostVibrant(box);
    // Blend 50/50
    const blended: RGB = {
      r: (avg.r + vib.r) / 2,
      g: (avg.g + vib.g) / 2,
      b: (avg.b + vib.b) / 2,
    };
    const hsl = rgbToHsl(blended);

    // Filter: need reasonable saturation and lightness
    if (hsl.s < 0.2 || hsl.l < 0.15 || hsl.l > 0.85) continue;

    // Saturation score (weight 3)
    const satScore = hsl.s;
    // Luminance score: prefer mid-brightness (weight 6)
    const lumScore = 1 - Math.abs(hsl.l - 0.5) * 2;
    // Distinctiveness: min HSL distance from top 3 dominant (weight 4)
    let minDist = 1;
    for (const d of dominant) {
      const dh = Math.min(Math.abs(hsl.h - d.h), 1 - Math.abs(hsl.h - d.h));
      const ds = Math.abs(hsl.s - d.s);
      const dl = Math.abs(hsl.l - d.l);
      const dist = Math.sqrt(dh * dh + ds * ds + dl * dl);
      if (dist < minDist) minDist = dist;
    }
    const distinctScore = Math.min(minDist * 2, 1);

    const score = (satScore * 3 + lumScore * 6 + distinctScore * 4) / 13;
    candidates.push({ rgb: blended, score });
  }

  if (candidates.length === 0) return null;

  // Pick the highest scoring candidate
  candidates.sort((a, b) => b.score - a.score);
  const winner = candidates[0].rgb;
  const winnerHsl = rgbToHsl(winner);

  // Enforce minimum lightness
  const finalL = Math.max(winnerHsl.l, MIN_LIGHTNESS);
  const final_ = hslToRgb(winnerHsl.h, winnerHsl.s, finalL);

  return [
    Math.round(final_.r * 255),
    Math.round(final_.g * 255),
    Math.round(final_.b * 255),
  ];
}
