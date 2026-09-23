import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { usePlaybackStore } from "../stores/playbackStore";
import { setVisualizerPresentation } from "../lib/commands";
import { useBlurColors } from "../lib/useBlurColors";
import UltraBlurBackground from "../components/UltraBlurBackground";
import FocusVisualizer from "../components/FocusVisualizer";

/**
 * Full-screen visualiser for touch devices: the ridge ("pulsar") at full
 * strength over the album-art gradient, in landscape. Opened from the
 * now-playing menu (`playbackStore.mobileVisualizerOpen`); a tap anywhere
 * (or Escape on a hardware keyboard) closes it.
 *
 * While it is mounted the native side holds its presentation
 * (`setVisualizerPresentation`): on iOS the interface turns to landscape
 * whatever the rotation lock says, and the screen stays awake while music
 * plays (a finished album or a pause lets the phone lock as usual).
 * Closing asks it to turn back and keeps the overlay up until the viewport
 * is portrait again, so the app behind it is never seen sideways. The
 * spectrum tap is `FocusVisualizer`'s: installed while this is on screen,
 * removed when it closes or the app is hidden.
 */

/** Longest the overlay waits for either turn before carrying on anyway. */
const TURN_TIMEOUT_MS = 1000;

const PORTRAIT_QUERY = "(orientation: portrait)";

/**
 * Presentation requests are chained so they reach the native side in the
 * order they were made: each command runs on its own backend task, and a
 * remount issues leave-then-enter back to back.
 */
let presentationRequests: Promise<void> = Promise.resolve();
function requestPresentation(active: boolean, keepAwake = false): void {
  presentationRequests = presentationRequests
    .then(() => setVisualizerPresentation(active, keepAwake))
    .catch((e) =>
      console.warn(`[visualizer] presentation ${active ? "enter" : "leave"} failed:`, e),
    );
}

// A fresh page holds no presentation. A reload, or a restarted web content
// process, runs no unmount cleanup, so one the previous page entered would
// otherwise stay held (landscape, screen awake) under a page with no
// visualiser open. Leaving when nothing is held is a no-op.
requestPresentation(false);

export default function MobileVisualizer() {
  const setOpen = usePlaybackStore((s) => s.setMobileVisualizerOpen);
  const playing = usePlaybackStore((s) => s.status === "playing");
  const colors = useBlurColors();
  const [closing, setClosing] = useState(false);
  // Only a viewport that was portrait when the visualiser opened has turns
  // to wait for (an iPad already in landscape stays put).
  const [openedPortrait] = useState(() => window.matchMedia(PORTRAIT_QUERY).matches);
  // Whether the turn to landscape has happened. A close asked for before
  // then waits for it: leaving mid-turn would find the viewport still
  // portrait, unmount at once and show the app behind turning sideways
  // and back.
  const [turned, setTurned] = useState(!openedPortrait);

  useEffect(() => () => requestPresentation(false), []);

  useEffect(() => {
    if (!closing) requestPresentation(true, playing);
  }, [closing, playing]);

  useEffect(() => {
    if (turned) return;
    const portrait = window.matchMedia(PORTRAIT_QUERY);
    const finish = () => setTurned(true);
    if (!portrait.matches) {
      finish();
      return;
    }
    const onChange = (e: MediaQueryListEvent) => {
      if (!e.matches) finish();
    };
    portrait.addEventListener("change", onChange);
    const timer = window.setTimeout(finish, TURN_TIMEOUT_MS);
    return () => {
      portrait.removeEventListener("change", onChange);
      window.clearTimeout(timer);
    };
  }, [turned]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      setClosing(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!closing || !turned) return;
    requestPresentation(false);
    const portrait = window.matchMedia(PORTRAIT_QUERY);
    if (!openedPortrait || portrait.matches) {
      setOpen(false);
      return;
    }
    const finish = () => setOpen(false);
    const onChange = (e: MediaQueryListEvent) => {
      if (e.matches) finish();
    };
    portrait.addEventListener("change", onChange);
    const timer = window.setTimeout(finish, TURN_TIMEOUT_MS);
    return () => {
      portrait.removeEventListener("change", onChange);
      window.clearTimeout(timer);
    };
  }, [closing, turned, openedPortrait, setOpen]);

  return createPortal(
    <div
      className="mobile-visualizer"
      role="button"
      aria-label="Close visualiser"
      onClick={() => setClosing(true)}
    >
      <UltraBlurBackground colors={colors} />
      <FocusVisualizer mode="ridge" subdued={false} fullScreen />
    </div>,
    document.body,
  );
}
