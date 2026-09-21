import { getCurrentWindow } from "@tauri-apps/api/window";
import { IconClose, IconMinimize, IconFullscreen, IconMaximize } from "./Icons";

const appWindow = getCurrentWindow();

/** True when running inside WKWebView on macOS. */
const IS_MACOS = navigator.userAgent.includes("Macintosh");

/**
 * Custom window controls + drag region.
 *
 * macOS (left-aligned): close, minimize, fullscreen. Green button
 * enters a fullscreen Space via setFullscreen. The Rust setup hook adds
 * NSWindowCollectionBehaviorFullScreenPrimary so setFullscreen works on
 * a `decorations:false` window.
 *
 * The macOS strip does NOT carry `data-tauri-drag-region`. Tauri's own
 * script handles a macOS double-click on mouseup by calling its internal
 * toggle-maximize command, which refuses to maximise unless the window's
 * standard zoom button is enabled — and a borderless window has no
 * standard buttons, so the maximise leg is a silent no-op (the
 * un-maximise leg still runs). The public `toggleMaximize` has no such
 * gate. Dragging and the double-click are therefore handled here, and
 * the attribute is left off so Tauri's handler can't fire a second toggle
 * on top of ours.
 *
 * Windows / Linux (right-aligned): minimize, maximize, close.
 * "Maximize" toggles maximise (not exclusive fullscreen) so the
 * controls stay visible. Tauri's drag attribute works as intended there.
 */
export default function TrafficLights() {
  if (IS_MACOS) {
    const handleFullscreen = async () => {
      const isFs = await appWindow.isFullscreen();
      await appWindow.setFullscreen(!isFs);
    };

    const onControl = (e: React.MouseEvent<HTMLDivElement>) =>
      !!(e.target as HTMLElement).closest("button");

    // First press starts the native window drag. A second press of a
    // double-click must not: AppKit's drag tracking would swallow the
    // mouseup that completes the double-click below.
    const handleMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0 || e.detail !== 1 || onControl(e)) return;
      e.preventDefault();
      appWindow.startDragging().catch(() => {});
    };

    // Toggle on the mouseup of the second click, mirroring the native
    // title-bar behaviour (and Tauri's own macOS approach): `detail` is
    // the OS click count, so this fires exactly once per double-click
    // even though the first mouseup may never reach the webview.
    const handleMouseUp = (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0 || e.detail !== 2 || onControl(e)) return;
      appWindow.toggleMaximize().catch(() => {});
    };

    return (
      <div className="drag-region" onMouseDown={handleMouseDown} onMouseUp={handleMouseUp}>
        <div className="traffic-lights">
          <button
            className="traffic-light tl-close"
            title="Close"
            onClick={() => appWindow.close()}
          >
            <IconClose size={10} />
          </button>
          <button
            className="traffic-light tl-minimize"
            title="Minimize"
            onClick={() => appWindow.minimize()}
          >
            <IconMinimize size={10} />
          </button>
          <button
            className="traffic-light tl-fullscreen"
            title="Toggle Full Screen"
            onClick={handleFullscreen}
          >
            <IconFullscreen size={10} />
          </button>
        </div>
      </div>
    );
  }

  // Windows / Linux: right-aligned, minimize → maximize → close.
  return (
    <div className="drag-region" data-tauri-drag-region>
      <div className="traffic-lights traffic-lights-right">
        <button
          className="traffic-light tl-minimize"
          title="Minimize"
          onClick={() => appWindow.minimize()}
        >
          <IconMinimize size={10} />
        </button>
        <button
          className="traffic-light tl-maximize"
          title="Maximize"
          onClick={() => appWindow.toggleMaximize()}
        >
          <IconMaximize size={10} />
        </button>
        <button className="traffic-light tl-close" title="Close" onClick={() => appWindow.close()}>
          <IconClose size={10} />
        </button>
      </div>
    </div>
  );
}
