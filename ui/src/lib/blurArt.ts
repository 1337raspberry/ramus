/**
 * Corner-colour extraction from album art for the UltraBlur background.
 *
 * The artwork is reduced to four colours — one per corner — which feed
 * the existing four-corner radial-gradient renderer in
 * `UltraBlurBackground` (smooth and banding-free by construction).
 * Rendering a low-res image of the art directly was tried and rejected:
 * any resolution high enough to keep regional detail reads as a
 * pixelated stretched image, and any blur strong enough to hide the
 * grid dilutes the colours.
 *
 * Extraction is spatial and dominance-weighted:
 *
 * 1. Downscale to a small intermediate and split into a GRID_SIZE ×
 *    GRID_SIZE cell grid. Each cell's colour is a chroma-weighted
 *    average of its pixels (weight `WEIGHT_FLOOR + chroma^WEIGHT_POWER`)
 *    so a small vivid region — a neon sign on a dark backdrop — wins its
 *    cell the way the eye reads it.
 * 2. Pool each corner quadrant down to one colour by picking its most
 *    SALIENT cell — scored by chroma × distinctiveness from the image's
 *    overall mean colour — blended with the quadrant's chroma-weighted
 *    average. Plain averaging fails here: on art that is 80% one hue
 *    with one distinct feature (a green sign on a red cover), every
 *    quadrant averages to the dominant hue and the feature vanishes,
 *    when it's precisely what the eye picks out of that region.
 *
 * Averaging is what makes this safe on (near-)monochrome art: every
 * pixel has ~zero chroma, both weighting stages degenerate to plain
 * means, and grey stays grey. JPEG compression noise cannot surface the
 * way it did with swatch-based extraction, because a handful of noisy
 * pixels carry negligible total weight. Colour is only ever
 * re-balanced, never invented.
 *
 * No tone adjustment happens here — `UltraBlurBackground` applies its
 * saturation/brightness pass to whatever corner colours it receives,
 * same as for the server-provided ones.
 */

import type { UltraBlurColors } from "./types";

/** Intermediate downscale size — pre-averages away pixel-level noise. */
const INTERMEDIATE_SIZE = 128;
/** Cell grid resolution. 4×4 keeps enough spatial separation that a
 * feature in the middle of one edge lands in the right corners. */
const GRID_SIZE = 4;
/**
 * Chroma-weight exponent + floor, used in both weighting stages. The
 * floor keeps monochrome input on the plain-average path; the exponent
 * controls how strongly vivid colours dominate their cell/corner.
 */
const WEIGHT_POWER = 2;
const WEIGHT_FLOOR = 0.05;
/**
 * How much of the corner colour comes from its most salient cell vs the
 * quadrant's chroma-weighted average. Full winner-take-all makes a
 * single odd cell too dominant; full averaging muddies opposing hues
 * (red + green → brown) and erases regional features.
 */
const WINNER_BLEND = 0.65;
/**
 * Brightness floor for CHROMATIC corner colours: a vivid feature picked
 * from shadow (e.g. a dim neon sign) reads as near-black next to a
 * bright neighbouring corner, so scale its max channel up to this
 * level. Gated on chroma so achromatic corners are untouched — dark
 * monochrome art must stay dark.
 */
const MIN_VIVID_LEVEL = 110;
const VIVID_CHROMA_GATE = 0.15;
/**
 * Chroma-dependent brightness CEILING — the counterpart of the floor.
 * Pale, washed-out colours (a white-ish sky) must be pulled down into
 * the dark-theme band or white UI text becomes unreadable over them,
 * while genuinely saturated colours (a bright red cover) may stay
 * bright. The allowed max channel scales from PALE_CEILING at zero
 * chroma up to VIVID_CEILING at CEILING_CHROMA_REF and beyond.
 */
const PALE_CEILING = 120;
const VIVID_CEILING = 210;
const CEILING_CHROMA_REF = 0.5;
/**
 * Final cap on perceptual brightness (Rec.601 luma). The channel-based
 * ceiling above is blind to hue: yellow at max-channel 210 is glaring
 * (luma ~190) while red at the same value stays dark (luma ~70), so a
 * saturated yellow cover sails through the chroma ceiling and makes
 * white UI text unreadable. Capping luma treats hues by how bright
 * they actually LOOK — bright yellows darken to olive (matching how
 * the reference players render them), reds and blues pass untouched.
 */
const LUMA_CAP = 120;
/**
 * Context mix: blend the pooled colour toward the quadrant's PLAIN
 * (unweighted) average, gated on hue similarity. Every earlier stage is
 * chroma-seeking, so on art where a saturated colour borders neutral
 * context (red splatter on white + black) the neutrals get filtered out
 * at every step and the corner comes out neon. Mixing the plain average
 * back in restores that muting context (red + black + white → brick) —
 * but ONLY when the pooled colour and the context share a hue. A
 * cross-hue winner (green sign in a red quadrant) is a genuine regional
 * feature and keeps its pop: the mix fades to zero as the hue gap
 * approaches CONTEXT_HUE_RANGE. Near-achromatic colours skip the hue
 * test (hue is meaningless there) and always take the full mix.
 */
const CONTEXT_MIX = 0.4;
const CONTEXT_HUE_RANGE = 90;
const CONTEXT_CHROMA_GATE = 0.07;
/**
 * Final chroma cap. The display pass multiplies saturation, so a
 * fully-saturated corner would get slammed to the gamut edge (neon).
 * Capping chroma here keeps the post-boost result rich but not harsh.
 */
const CHROMA_CAP = 0.45;

interface Rgb {
  r: number;
  g: number;
  b: number;
}

function chromaWeight(r: number, g: number, b: number): number {
  const chroma = (Math.max(r, g, b) - Math.min(r, g, b)) / 255;
  return WEIGHT_FLOOR + Math.pow(chroma, WEIGHT_POWER);
}

/** Chroma-weighted downscale: intermediate pixels → GRID_SIZE² cells. */
function weightedCells(src: Uint8ClampedArray, srcSize: number): Rgb[] {
  const out: Rgb[] = [];
  const cellPx = srcSize / GRID_SIZE;
  for (let cy = 0; cy < GRID_SIZE; cy++) {
    for (let cx = 0; cx < GRID_SIZE; cx++) {
      let sr = 0;
      let sg = 0;
      let sb = 0;
      let sw = 0;
      const x0 = Math.floor(cx * cellPx);
      const x1 = Math.floor((cx + 1) * cellPx);
      const y0 = Math.floor(cy * cellPx);
      const y1 = Math.floor((cy + 1) * cellPx);
      for (let y = y0; y < y1; y++) {
        for (let x = x0; x < x1; x++) {
          const i = (y * srcSize + x) * 4;
          const w = chromaWeight(src[i], src[i + 1], src[i + 2]);
          sr += src[i] * w;
          sg += src[i + 1] * w;
          sb += src[i + 2] * w;
          sw += w;
        }
      }
      out.push({ r: sr / sw, g: sg / sw, b: sb / sw });
    }
  }
  return out;
}

function chromaOf(c: Rgb): number {
  return (Math.max(c.r, c.g, c.b) - Math.min(c.r, c.g, c.b)) / 255;
}

/** Hue angle in degrees, 0..360. Meaningless for achromatic colours —
 * callers must gate on chroma first. */
function hueDeg(c: Rgb): number {
  const max = Math.max(c.r, c.g, c.b);
  const min = Math.min(c.r, c.g, c.b);
  const d = max - min;
  if (d === 0) return 0;
  let h: number;
  if (max === c.r) h = ((c.g - c.b) / d) % 6;
  else if (max === c.g) h = (c.b - c.r) / d + 2;
  else h = (c.r - c.g) / d + 4;
  return (h * 60 + 360) % 360;
}

/** Normalised colour distance, 0..1. */
function colorDist(a: Rgb, b: Rgb): number {
  const dr = (a.r - b.r) / 255;
  const dg = (a.g - b.g) / 255;
  const db = (a.b - b.b) / 255;
  return Math.sqrt(dr * dr + dg * dg + db * db) / Math.sqrt(3);
}

/**
 * Pool a corner quadrant's cells into one colour: the most salient cell
 * (chromatic AND distinct from the image's mean — i.e. what the eye
 * picks out of that region) blended with the quadrant's chroma-weighted
 * average. On monochrome art every score is ~equal and every cell is
 * ~the average, so the pick degenerates to the plain average — safe.
 */
function poolCorner(cells: Rgb[], xs: number[], ys: number[], mean: Rgb): Rgb {
  let sr = 0;
  let sg = 0;
  let sb = 0;
  let sw = 0;
  let pr = 0;
  let pg = 0;
  let pb = 0;
  let n = 0;
  let winner: Rgb | null = null;
  let best = -1;
  for (const y of ys) {
    for (const x of xs) {
      const c = cells[y * GRID_SIZE + x];
      const w = chromaWeight(c.r, c.g, c.b);
      sr += c.r * w;
      sg += c.g * w;
      sb += c.b * w;
      sw += w;
      pr += c.r;
      pg += c.g;
      pb += c.b;
      n += 1;
      const score = (0.1 + chromaOf(c)) * (0.15 + colorDist(c, mean));
      if (score > best) {
        best = score;
        winner = c;
      }
    }
  }
  const avg = { r: sr / sw, g: sg / sw, b: sb / sw };
  const plain = { r: pr / n, g: pg / n, b: pb / n };
  let out = !winner
    ? avg
    : {
        r: winner.r * WINNER_BLEND + avg.r * (1 - WINNER_BLEND),
        g: winner.g * WINNER_BLEND + avg.g * (1 - WINNER_BLEND),
        b: winner.b * WINNER_BLEND + avg.b * (1 - WINNER_BLEND),
      };

  // Hue-gated context mix (see CONTEXT_MIX).
  let ctx = CONTEXT_MIX;
  if (chromaOf(out) >= CONTEXT_CHROMA_GATE && chromaOf(plain) >= CONTEXT_CHROMA_GATE) {
    let hueGap = Math.abs(hueDeg(out) - hueDeg(plain));
    hueGap = Math.min(hueGap, 360 - hueGap);
    ctx = CONTEXT_MIX * Math.max(0, 1 - hueGap / CONTEXT_HUE_RANGE);
  }
  out = {
    r: out.r * (1 - ctx) + plain.r * ctx,
    g: out.g * (1 - ctx) + plain.g * ctx,
    b: out.b * (1 - ctx) + plain.b * ctx,
  };

  // Brightness floor for vivid-but-dark colours.
  const max = Math.max(out.r, out.g, out.b);
  const chroma = chromaOf(out);
  if (chroma > VIVID_CHROMA_GATE && max > 0 && max < MIN_VIVID_LEVEL) {
    const scale = MIN_VIVID_LEVEL / max;
    out = { r: out.r * scale, g: out.g * scale, b: out.b * scale };
  } else {
    // Chroma-scaled channel ceiling for pale colours.
    const ceiling =
      PALE_CEILING + (VIVID_CEILING - PALE_CEILING) * Math.min(1, chroma / CEILING_CHROMA_REF);
    if (max > ceiling) {
      const scale = ceiling / max;
      out = { r: out.r * scale, g: out.g * scale, b: out.b * scale };
    }
  }

  // Perceptual brightness cap (luminous hues like yellow/cyan).
  const luma = 0.299 * out.r + 0.587 * out.g + 0.114 * out.b;
  if (luma > LUMA_CAP) {
    const scale = LUMA_CAP / luma;
    out = { r: out.r * scale, g: out.g * scale, b: out.b * scale };
  }

  // Chroma cap so the display saturation boost can't reach neon.
  const finalChroma = chromaOf(out);
  if (finalChroma > CHROMA_CAP) {
    const grey = (out.r + out.g + out.b) / 3;
    const t = CHROMA_CAP / finalChroma;
    out = {
      r: grey + (out.r - grey) * t,
      g: grey + (out.g - grey) * t,
      b: grey + (out.b - grey) * t,
    };
  }
  return out;
}

function toHex(c: Rgb): string {
  const h = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v)))
      .toString(16)
      .padStart(2, "0");
  // No leading '#', matching the server-provided corner colours.
  return `${h(c.r)}${h(c.g)}${h(c.b)}`;
}

/**
 * Extract the four corner colours from a decoded album-art image, or
 * null if the image can't be read (decode failure, zero-size, etc.).
 * Synchronous: all canvas work happens before this returns.
 */
export function extractCornerColors(img: HTMLImageElement): UltraBlurColors | null {
  try {
    if (!img.naturalWidth || !img.naturalHeight) return null;

    const canvas = document.createElement("canvas");
    canvas.width = INTERMEDIATE_SIZE;
    canvas.height = INTERMEDIATE_SIZE;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(img, 0, 0, INTERMEDIATE_SIZE, INTERMEDIATE_SIZE);

    const data = ctx.getImageData(0, 0, INTERMEDIATE_SIZE, INTERMEDIATE_SIZE);
    const cells = weightedCells(data.data, INTERMEDIATE_SIZE);

    const mean = {
      r: cells.reduce((s, c) => s + c.r, 0) / cells.length,
      g: cells.reduce((s, c) => s + c.g, 0) / cells.length,
      b: cells.reduce((s, c) => s + c.b, 0) / cells.length,
    };

    const lo = [0, 1];
    const hi = [GRID_SIZE - 2, GRID_SIZE - 1];
    return {
      topLeft: toHex(poolCorner(cells, lo, lo, mean)),
      topRight: toHex(poolCorner(cells, hi, lo, mean)),
      bottomLeft: toHex(poolCorner(cells, lo, hi, mean)),
      bottomRight: toHex(poolCorner(cells, hi, hi, mean)),
    };
  } catch {
    return null;
  }
}
