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

import { setMediaAccent } from "./commands";

let lastR = -1;
let lastG = -1;
let lastB = -1;

/**
 * Write accent CSS custom properties AND forward the colour to the OS
 * media controls. Dedupes on exact RGB so rapid palette re-extractions
 * don't spam the Kotlin bridge with identical updates.
 */
export function applyAccent(r: number, g: number, b: number): void {
  const root = document.documentElement;
  root.style.setProperty("--accent-r", String(r));
  root.style.setProperty("--accent-g", String(g));
  root.style.setProperty("--accent-b", String(b));

  if (r === lastR && g === lastG && b === lastB) return;
  lastR = r;
  lastG = g;
  lastB = b;
  setMediaAccent(r, g, b).catch(() => {
    // Best-effort cosmetic IPC; swallow so palette extraction never
    // produces an uncaught rejection in the console.
  });
}
