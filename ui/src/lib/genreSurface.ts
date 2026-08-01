import { create } from "zustand";

import { fetchGenreMetadata, peekGenreMetadata } from "./genreMetadataCache";
import { useGenreInfoStore } from "../stores/genreInfoStore";

/**
 * Desktop genre-surface interactions: right-click opens the genre-info modal,
 * and resting the pointer on a genre for `HOVER_INTENT_MS` without moving
 * opens a small summary card.
 *
 * The hover half is a single app-wide controller rather than per-element
 * state: `genreSurfaceHandlers` is a plain function (usable inside
 * virtualized row maps where hooks can't run), all transient state lives in
 * this module, and one globally-mounted <GenreHoverCard/> renders whatever
 * the store says is open. The card itself is pointer-events: none, so it can
 * never steal events from the anchor beneath it.
 */

export const HOVER_INTENT_MS = 400;

interface GenreHoverState {
  genre: string | null;
  anchor: HTMLElement | null;
}

export const useGenreHoverStore = create<GenreHoverState>(() => ({
  genre: null,
  anchor: null,
}));

// Touch-first devices get long-press → sheet instead; the hover machinery
// would only misfire on synthesized mouse events.
const coarsePointer =
  typeof window !== "undefined" && window.matchMedia("(pointer: coarse)").matches;

type Phase = "idle" | "armed" | "open";

let phase: Phase = "idle";
let timerId: number | null = null;
let armedGenre = "";
let armedAnchor: HTMLElement | null = null;
// Set when the dwell timer fired before the metadata fetch settled; the
// fetch's continuation re-runs the open attempt.
let pendingOpen = false;
let listenersInstalled = false;

/** Single funnel for every close/cancel path; safe to call in any phase. */
function reset() {
  if (timerId !== null) {
    window.clearTimeout(timerId);
    timerId = null;
  }
  pendingOpen = false;
  phase = "idle";
  armedAnchor = null;
  removeGlobalListeners();
  if (useGenreHoverStore.getState().genre !== null) {
    useGenreHoverStore.setState({ genre: null, anchor: null });
  }
}

function onGlobalClose() {
  reset();
}

// Deliberately no preventDefault: the card is informational, so Escape must
// still reach whatever overlay/focus-mode handler it was aimed at.
function onGlobalKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") reset();
}

// Safety net for anchors that vanish under a stationary pointer (virtualized
// rows, re-rendered pills): mouseleave never fires for unmounted elements,
// but the next pointer movement lands outside the stale anchor.
function onGlobalMouseOver(e: MouseEvent) {
  if (phase !== "open") return;
  const anchor = armedAnchor;
  if (!anchor || !anchor.isConnected || !(e.target instanceof Node) || !anchor.contains(e.target)) {
    reset();
  }
}

function addGlobalListeners() {
  if (listenersInstalled) return;
  listenersInstalled = true;
  // Capture-phase scroll catches every scroll container, including the
  // virtualized genre tree; the rest close the card before anything that
  // could move or re-purpose the anchor.
  document.addEventListener("scroll", onGlobalClose, true);
  window.addEventListener("wheel", onGlobalClose, { passive: true });
  window.addEventListener("resize", onGlobalClose);
  window.addEventListener("blur", onGlobalClose);
  window.addEventListener("mousedown", onGlobalClose, true);
  window.addEventListener("contextmenu", onGlobalClose, true);
  window.addEventListener("keydown", onGlobalKeydown);
  document.addEventListener("mouseover", onGlobalMouseOver, true);
}

function removeGlobalListeners() {
  if (!listenersInstalled) return;
  listenersInstalled = false;
  document.removeEventListener("scroll", onGlobalClose, true);
  window.removeEventListener("wheel", onGlobalClose);
  window.removeEventListener("resize", onGlobalClose);
  window.removeEventListener("blur", onGlobalClose);
  window.removeEventListener("mousedown", onGlobalClose, true);
  window.removeEventListener("contextmenu", onGlobalClose, true);
  window.removeEventListener("keydown", onGlobalKeydown);
  document.removeEventListener("mouseover", onGlobalMouseOver, true);
}

function restartDwellTimer() {
  if (timerId !== null) window.clearTimeout(timerId);
  timerId = window.setTimeout(tryOpen, HOVER_INTENT_MS);
}

function tryOpen() {
  timerId = null;
  if (phase !== "armed") return;
  const meta = peekGenreMetadata(armedGenre);
  if (meta === undefined) {
    // Dwell satisfied but the fetch hasn't settled — open as soon as it does.
    pendingOpen = true;
    return;
  }
  // Nothing worth a card (no metadata, or metadata without a summary):
  // stay silent. Right-click still opens the full modal.
  if (!meta?.shortSummary || !armedAnchor?.isConnected) {
    reset();
    return;
  }
  phase = "open";
  useGenreHoverStore.setState({ genre: armedGenre, anchor: armedAnchor });
}

function enter(genre: string, anchor: HTMLElement) {
  if (phase === "open" && anchor === armedAnchor) return;
  reset();
  // Known-empty from a previous fetch: never arm.
  const peeked = peekGenreMetadata(genre);
  if (peeked !== undefined && !peeked?.shortSummary) return;
  phase = "armed";
  armedGenre = genre;
  armedAnchor = anchor;
  addGlobalListeners();
  restartDwellTimer();
  // Prefetch so the card has its summary the moment the dwell elapses.
  void fetchGenreMetadata(genre).then(() => {
    if (pendingOpen && phase === "armed" && armedGenre === genre) {
      pendingOpen = false;
      tryOpen();
    }
  });
}

function move() {
  // Hover-intent: the card only opens once the pointer has been stationary
  // for the full dwell. Once open it stays put while the pointer remains
  // inside the anchor.
  if (phase === "armed") restartDwellTimer();
}

function leave() {
  reset();
}

export interface GenreSurfaceHandlers {
  onContextMenu: (e: React.MouseEvent) => void;
  onMouseEnter?: (e: React.MouseEvent) => void;
  onMouseMove?: () => void;
  onMouseLeave?: () => void;
}

/**
 * Spreadable handlers for any desktop element representing a genre.
 * `stopPropagation` on right-click lets a container and a child both carry
 * handlers without double-opening the modal.
 *
 * The modal's drill stack resets on the store's null → name transition, so
 * these handlers must stay unreachable while the modal itself is open — its
 * backdrop guarantees that today.
 */
export function genreSurfaceHandlers(genre: string): GenreSurfaceHandlers {
  return {
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      reset();
      useGenreInfoStore.getState().open(genre);
    },
    onMouseEnter: coarsePointer
      ? undefined
      : (e: React.MouseEvent) => enter(genre, e.currentTarget as HTMLElement),
    onMouseMove: coarsePointer ? undefined : move,
    onMouseLeave: coarsePointer ? undefined : leave,
  };
}
