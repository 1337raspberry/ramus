/**
 * Color extraction from album art using node-vibrant.
 * Provides categorized swatches (Vibrant, Muted, DarkVibrant, etc.)
 * for both accent color and ultrablur background.
 */

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

const MIN_ACCENT_LIGHTNESS = 0.55;

/** Extract a 6-swatch palette from an HTMLImageElement via node-vibrant. */
export async function extractPalette(img: HTMLImageElement): Promise<VibrantPalette | null> {
  try {
    const palette = await Vibrant.from(img).getPalette();
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
    // Boost lightness while preserving hue
    const factor = MIN_ACCENT_LIGHTNESS / Math.max(l, 0.01);
    const boosted: [number, number, number] = [
      Math.min(255, Math.round(rgb[0] * factor)),
      Math.min(255, Math.round(rgb[1] * factor)),
      Math.min(255, Math.round(rgb[2] * factor)),
    ];
    // If boost produced a usable color (not near-black), use it
    if (Math.max(...boosted) > 50) return boosted;
    // Otherwise try next candidate
  }
  return [140, 140, 140];
}

/** Map palette swatches to 4 ultrablur corner colors. */
export function blurColorsFromPalette(p: VibrantPalette): UltraBlurColors {
  // Pick the best available swatch for each corner, with fallback chains
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

function hexToRgb(hex: string): [number, number, number] {
  const cleaned = hex.startsWith("#") ? hex.slice(1) : hex;
  const value = parseInt(cleaned, 16);
  if (isNaN(value)) return [0, 0, 0];
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff];
}
