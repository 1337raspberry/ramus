import { setWebviewVisible } from "./commands";

/**
 * Whether the backend treats the webview as on screen. Two things hide it:
 * the document itself being hidden, and a native view covering the page
 * (the iOS full-screen visualiser). While either holds, position and
 * download-progress ticks stop. A backgrounded page would otherwise be woken
 * by each one, and a covered page would redraw its seek bar and re-render the
 * now-playing sheet on every tick under a view that hides all of it. Showing
 * the page again sends the position it missed.
 *
 * Reports are chained so they reach the backend in the order they happened:
 * each command runs on its own backend task, and a quick hide/show pair
 * landing reversed would leave the backend holding back position ticks from
 * a page that is on screen.
 */
let pageVisible = true;
let coveredByNativeView = false;
let reportedVisible = true;
let ticksResumedAt = -Infinity;
let reports: Promise<void> = Promise.resolve();

function report(): void {
  const visible = pageVisible && !coveredByNativeView;
  if (visible && !reportedVisible) ticksResumedAt = performance.now();
  reportedVisible = visible;
  reports = reports.then(() => setWebviewVisible(visible)).catch(() => {});
}

/** Whether position ticks are being held back from the page. Their absence
 * then says nothing about playback, so it must not read as a stall. */
export function positionTicksHeld(): boolean {
  return !reportedVisible;
}

/** When held ticks were last let through again (`performance.now()` time).
 * The position the page missed is still in flight at that moment, so a
 * staleness check counts from here as well as from the last tick. */
export function positionTicksResumedAt(): number {
  return ticksResumedAt;
}

/** The document's own visibility: reported on load and on every edge. */
export function reportPageVisible(visible: boolean): void {
  pageVisible = visible;
  report();
}

/** Whether a native view covers the page. */
export function reportCoveredByNativeView(covered: boolean): void {
  if (coveredByNativeView === covered) return;
  coveredByNativeView = covered;
  report();
}
