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
 * closes it.
 *
 * While it is mounted the native side holds its presentation
 * (`setVisualizerPresentation`): on iOS the interface turns to landscape
 * whatever the rotation lock says, and the screen stays awake. Closing
 * asks it to turn back and keeps the overlay up until the viewport is
 * portrait again, so the app behind it is never seen sideways. The
 * spectrum tap is `FocusVisualizer`'s: installed while this is on screen,
 * removed when it closes or the app is hidden.
 */

/** Longest the overlay waits for the turn back before closing anyway. */
const TURN_BACK_TIMEOUT_MS = 1000;

const PORTRAIT_QUERY = "(orientation: portrait)";

/**
 * Presentation requests are chained so they reach the native side in the
 * order they were made: each command runs on its own backend task, and a
 * remount issues leave-then-enter back to back.
 */
let presentationRequests: Promise<void> = Promise.resolve();
function requestPresentation(active: boolean): void {
  presentationRequests = presentationRequests
    .then(() => setVisualizerPresentation(active))
    .catch((e) =>
      console.warn(`[visualizer] presentation ${active ? "enter" : "leave"} failed:`, e),
    );
}

export default function MobileVisualizer() {
  const setOpen = usePlaybackStore((s) => s.setMobileVisualizerOpen);
  const colors = useBlurColors();
  const [closing, setClosing] = useState(false);
  // Only a viewport that was portrait when the visualiser opened has a
  // turn back to wait for (an iPad already in landscape stays put).
  const [openedPortrait] = useState(() => window.matchMedia(PORTRAIT_QUERY).matches);

  useEffect(() => {
    requestPresentation(true);
    return () => requestPresentation(false);
  }, []);

  useEffect(() => {
    if (!closing) return;
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
    const timer = window.setTimeout(finish, TURN_BACK_TIMEOUT_MS);
    return () => {
      portrait.removeEventListener("change", onChange);
      window.clearTimeout(timer);
    };
  }, [closing, openedPortrait, setOpen]);

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
