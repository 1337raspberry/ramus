/** Color extraction from album art using node-vibrant. */

import { Vibrant } from "node-vibrant/browser";
import type { UltraBlurColors } from "./types";

export interface VibrantPalette {
  vibrant: string | null;
  darkVibrant: string | null;
  lightVibrant: string | null;
  muted: string | null;
  darkMuted: string | null;
  lightMuted: string | null;
}

// Override node-vibrant defaults (quality 5, maxColorCount 64): lower quality
// = more pixels sampled; higher maxColorCount surfaces small vivid regions
// the default quantiser misses on album art.
const QUALITY = 2;
const MAX_COLOR_COUNT = 176;

const MIN_ACCENT_LIGHTNESS = 0.55;

/**
 * Ensure node-vibrant sees the image at its intrinsic pixel size.
 *
 * `@vibrant/image-browser` sizes its canvas from `img.width` / `img.height`,
 * and per the HTML spec those reflect the **rendered** CSS dimensions for an
 * in-DOM <img>. That means passing a rendered <img> causes extraction to run
 * on a tiny (~300×300) canvas instead of the full resolution, which loses
 * small-but-vivid regions of the album art. An off-DOM <img> instead falls
 * back to `naturalWidth`/`naturalHeight`, which is what we want.
 *
 * This helper detaches the image: if the input is already off-DOM we return
 * it as-is; otherwise we create a new Image() with the same source and wait
 * for it to load (instant browser-cache hit in practice).
 */
function detachImage(img: HTMLImageElement): Promise<HTMLImageElement> {
  if (!img.isConnected) return Promise.resolve(img);
  const src = img.src;
  return new Promise((resolve, reject) => {
    const fresh = new Image();
    fresh.crossOrigin = img.crossOrigin ?? "anonymous";
    fresh.onload = () => resolve(fresh);
    fresh.onerror = () => reject(new Error(`Failed to reload image: ${src}`));
    fresh.src = src;
  });
}

/** Extract a 6-swatch palette from an HTMLImageElement via node-vibrant. */
export async function extractPalette(img: HTMLImageElement): Promise<VibrantPalette | null> {
  try {
    const source = await detachImage(img);
    const palette = await Vibrant.from(source)
      .quality(QUALITY)
      .maxColorCount(MAX_COLOR_COUNT)
      .getPalette();
    return {
      vibrant: palette.Vibrant?.hex ?? null,
      darkVibrant: palette.DarkVibrant?.hex ?? null,
      lightVibrant: palette.LightVibrant?.hex ?? null,
      muted: palette.Muted?.hex ?? null,
      darkMuted: palette.DarkMuted?.hex ?? null,
      lightMuted: palette.LightMuted?.hex ?? null,
    };
  } catch {
    return null;
  }
}

/** Pick the best accent color from a palette. Enforces minimum lightness for UI visibility. */
export function accentFromPalette(p: VibrantPalette): [number, number, number] {
  const candidates = [p.vibrant, p.lightVibrant, p.muted, p.darkVibrant, p.lightMuted, p.darkMuted];
  for (const hex of candidates) {
    if (!hex) continue;
    const rgb = hexToRgb(hex);
    const max = Math.max(rgb[0], rgb[1], rgb[2]) / 255;
    const min = Math.min(rgb[0], rgb[1], rgb[2]) / 255;
    const l = (max + min) / 2;
    if (l >= MIN_ACCENT_LIGHTNESS) return rgb;
    // Boost lightness while preserving hue.
    const factor = MIN_ACCENT_LIGHTNESS / Math.max(l, 0.01);
    const boosted: [number, number, number] = [
      Math.min(255, Math.round(rgb[0] * factor)),
      Math.min(255, Math.round(rgb[1] * factor)),
      Math.min(255, Math.round(rgb[2] * factor)),
    ];
    if (Math.max(...boosted) > 50) return boosted;
  }
  return [140, 140, 140];
}

/** Map palette swatches to 4 ultrablur corner colors. */
export function blurColorsFromPalette(p: VibrantPalette): UltraBlurColors {
  const fallback = p.darkMuted ?? p.muted ?? p.darkVibrant ?? "333333";
  return {
    topLeft: stripHash(p.darkMuted ?? p.muted ?? fallback),
    topRight: stripHash(p.darkVibrant ?? p.darkMuted ?? fallback),
    bottomLeft: stripHash(p.muted ?? p.darkMuted ?? fallback),
    bottomRight: stripHash(p.darkMuted ?? p.muted ?? fallback),
  };
}

function stripHash(hex: string): string {
  return hex.startsWith("#") ? hex.slice(1) : hex;
}

export function hexToRgb(hex: string): [number, number, number] {
  const cleaned = hex.startsWith("#") ? hex.slice(1) : hex;
  const value = parseInt(cleaned, 16);
  if (isNaN(value)) return [0, 0, 0];
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff];
}
