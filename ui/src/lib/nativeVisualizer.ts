import {
  hideNativeVisualizer,
  showNativeVisualizer,
  updateNativeVisualizer,
  type NativeBackdropPayload,
} from "./commands";
import { adjustedRgb, readUltraBlurOpacity } from "./ultraBlurTone";
import type { UltraBlurColors } from "./types";
import { VISUALIZER_PARAMS } from "./visualizerParams";
import { reportCoveredByNativeView } from "./webviewVisibility";

/**
 * The native full-screen visualiser (iOS), as the page drives it: open on
 * the overlay's mount, colour and play-state updates while it is up, close
 * on unmount. Requests are chained so they reach the backend in the order
 * they were made — a remount issues close-then-open back to back, and each
 * command runs on its own backend task. `shown` is read inside the chain,
 * so an update queued behind an open applies once that open has landed and
 * one queued behind a close is dropped.
 */
let requests: Promise<unknown> = Promise.resolve();
let shown = false;

function enqueue<T>(run: () => Promise<T>): Promise<T> {
  const next = requests.then(run, run);
  requests = next.catch(() => {});
  return next;
}

// A fresh page shows nothing natively. A reload, or a restarted web content
// process, runs no unmount cleanup, so a view the previous page opened would
// otherwise stay over a page with no overlay to close it. Closing when
// nothing is showing is a no-op.
enqueue(() => hideNativeVisualizer()).catch(() => {});

/** The backdrop for `colors`, toned as the page's own backdrop tones them. */
export function nativeBackdrop(colors: UltraBlurColors): NativeBackdropPayload {
  return {
    topLeft: adjustedRgb(colors.topLeft),
    topRight: adjustedRgb(colors.topRight),
    bottomLeft: adjustedRgb(colors.bottomLeft),
    bottomRight: adjustedRgb(colors.bottomRight),
    opacity: readUltraBlurOpacity(),
  };
}

/** Open the native visualiser; resolves false where it can't draw. */
export function openNativeVisualizer(colors: UltraBlurColors, playing: boolean): Promise<boolean> {
  return enqueue(async () => {
    try {
      await showNativeVisualizer({ ...VISUALIZER_PARAMS }, nativeBackdrop(colors), playing);
      shown = true;
      // The native view covers the whole page: stop the page's position
      // ticks until it closes.
      reportCoveredByNativeView(true);
      return true;
    } catch (e) {
      console.info("[visualizer] drawing in the page:", e);
      return false;
    }
  });
}

/** Send colour or play-state changes, or a frame clear, while it is up. */
export function updateNativeVisualizerState(update: {
  backdrop?: NativeBackdropPayload;
  playing?: boolean;
  clearFrames?: boolean;
}): void {
  enqueue(async () => {
    if (!shown) return;
    await updateNativeVisualizer(update);
  }).catch((e) => console.warn("[visualizer] native update failed:", e));
}

/** Take the native visualiser down. */
export function closeNativeVisualizer(): void {
  enqueue(async () => {
    if (!shown) return;
    shown = false;
    // Uncovered first, so the position the page missed is on its way before
    // the page reappears.
    reportCoveredByNativeView(false);
    await hideNativeVisualizer();
  }).catch((e) => console.warn("[visualizer] native close failed:", e));
}

/** Drop the native visualiser's frames and clock (playback stopped, queue
 * cleared), as `clearSpectrumRing` does for the page's own ring. */
export function clearNativeVisualizerFrames(): void {
  updateNativeVisualizerState({ clearFrames: true });
}
