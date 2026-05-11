// Centralised accent-colour sink. The frontend extracts the accent from
// album art via `accentFromPalette` in several places (playback store,
// Now Playing views, mobile sheet). Historically each site wrote the
// three `--accent-*` CSS variables on its own; this helper keeps that
// working AND pushes the same value down to the OS media widget so the
// Android lock-screen notification can colorize itself to match.
//
// The IPC is fire-and-forget — desktop + iOS accept the call and no-op,
// Android hops via `spawn_blocking` so the bridge round-trip can't block
// the UI. Dropping the promise is safe; accent is best-effort cosmetic
// state.
//
// Also home for the brand-default palette constants used when album-art
// extraction is either unavailable (loading, onboarding, between
// tracks) or actively disabled by the `keepDefaultColours` setting.

import { setMediaAccent } from "./commands";
import { useSettingsStore } from "../stores/settingsStore";

/** Brand accent (`#f16280`) — used when no album art is driving the UI
 *  or when the user has opted into keeping the default colours. */
export const DEFAULT_ACCENT: [number, number, number] = [0xf1, 0x62, 0x80];

/** Brand UltraBlur corners — RGB-scaled to ~55% of the full-strength
 *  brand pinks so the gradient stays comfortable when it fills the
 *  whole viewport. */
export const DEFAULT_BLUR_COLORS = {
  topLeft: "853e43",
  topRight: "875147",
  bottomLeft: "853646",
  bottomRight: "854256",
};

let lastR = -1;
let lastG = -1;
let lastB = -1;

/**
 * Write accent CSS custom properties AND forward the colour to the OS
 * media controls. Dedupes on exact RGB so rapid palette re-extractions
 * don't spam the Kotlin bridge with identical updates.
 *
 * If the `keepDefaultColours` setting is on, the requested colour is
 * overridden with the brand default — so every accent call site (album
 * art extraction in the playback store, Now Playing views, etc.) is
 * neutralised at the funnel rather than needing per-caller gates.
 */
export function applyAccent(r: number, g: number, b: number): void {
  if (useSettingsStore.getState().keepDefaultColours) {
    [r, g, b] = DEFAULT_ACCENT;
  }
  if (r === lastR && g === lastG && b === lastB) return;
  lastR = r;
  lastG = g;
  lastB = b;

  const root = document.documentElement;
  root.style.setProperty("--accent-r", String(r));
  root.style.setProperty("--accent-g", String(g));
  root.style.setProperty("--accent-b", String(b));

  setMediaAccent(r, g, b).catch(() => {
    // Best-effort cosmetic IPC; swallow so palette extraction never
    // produces an uncaught rejection in the console.
  });
}
