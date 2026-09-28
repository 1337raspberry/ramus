import { isHDR } from "./hdr";
import { hexToRgb } from "./vibrantColor";
import type { Rgb } from "./ultraBlurField";

/**
 * The backdrop's baked tone adjustments. Saturation is a per-channel blend
 * toward the RGB mean (sRGB grey, not perceptually uniform, but
 * deterministic). Brightness is a scalar multiply clamped to [0, 255].
 * Order: saturation then brightness. Shared by the page's own backdrop and
 * the colours sent to the native visualiser.
 */
export const BRIGHTNESS = isHDR ? 0.9 : 1.0;
/* Tuned together with the extraction-side CHROMA_CAP in blurArt.ts —
 * the boost revives dull server-fallback colours, while the cap stops
 * already-saturated extracted colours from being pushed to neon. */
export const SATURATION = isHDR ? 1.05 : 1.3;

export function adjustedRgb(hex: string): Rgb {
  const [r, g, b] = hexToRgb(hex);
  const grey = (r + g + b) / 3;
  const clamp = (c: number) =>
    Math.max(0, Math.min(255, Math.round((grey + (c - grey) * SATURATION) * BRIGHTNESS)));
  return [clamp(r), clamp(g), clamp(b)];
}

/** Overall dim, from `--ultrablur-opacity` (raised on SDR screens). */
export function readUltraBlurOpacity(el: Element = document.documentElement): number {
  const v = parseFloat(getComputedStyle(el).getPropertyValue("--ultrablur-opacity"));
  return Number.isFinite(v) ? v : 0.8;
}
