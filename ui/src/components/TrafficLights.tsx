import { getCurrentWindow } from "@tauri-apps/api/window";
import { IconClose, IconMinimize, IconFullscreen, IconMaximize } from "./Icons";

const appWindow = getCurrentWindow();

/** True when running inside WKWebView on macOS. */
const IS_MACOS = navigator.userAgent.includes("Macintosh");

/**
 * Custom window controls + drag region.
 *
 * macOS (left-aligned): close, minimize, fullscreen. Green button
 * enters a fullscreen Space via setFullscreen. Double-click-to-zoom is
 * handled natively by `data-tauri-drag-region` — adding a JS handler
 * causes a double-fire bounce. The Rust setup hook adds
 * NSWindowCollectionBehaviorFullScreenPrimary so setFullscreen works on
 * a `decorations:false` window.
 *
 * Windows / Linux (right-aligned): minimize, maximize, close.
 * "Maximize" toggles maximise (not exclusive fullscreen) so the
 * controls stay visible.
 */
export default function TrafficLights() {
  if (IS_MACOS) {
    const handleFullscreen = async () => {
      const isFs = await appWindow.isFullscreen();
      await appWindow.setFullscreen(!isFs);
    };

    return (
      <div className="drag-region" data-tauri-drag-region>
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
