import { createPortal } from "react-dom";
import { dismissKeyboard } from "../lib/commands";

interface Props {
  /** Whether a text field on the surface below currently has focus. */
  visible: boolean;
}

/**
 * A "Done" strip pinned directly above the software keyboard.
 *
 * The keyboard is drawn by the OS, so a page can't add a key to it. iOS
 * normally offers an *input accessory view* — the native toolbar docked
 * above the keyboard that would carry Done — but the app swizzles that away
 * so it can't fight the native search bar, which leaves a focused web input
 * with no dismiss affordance at all.
 *
 * So the button is drawn in the page and the dismissal itself is routed
 * through the native bridge: WKWebView does not reliably hide the keyboard
 * for a web-side `blur()`. `blur()` is still called for the platforms where
 * the bridge is a no-op (Android, desktop), where it does work.
 *
 * Position rides `--keyboard-inset`, the keyboard's overlap with the webview
 * pushed in from the host — the keyboard slides *over* the page without
 * resizing it, and `visualViewport` doesn't report the occlusion, so this
 * variable is the only way the page can know where the keyboard's top edge
 * is.
 */
export default function KeyboardDoneBar({ visible }: Props) {
  if (!visible) return null;

  return createPortal(
    // Focus alone doesn't mean a keyboard is on screen — a hardware keyboard
    // (simulator, iPad, Bluetooth) leaves the field focused with nothing
    // covering the page, and the bar would strand itself at the bottom of the
    // screen. The outer element collapses itself to nothing whenever
    // --keyboard-inset is 0, so the bar exists only while something is
    // actually being covered up. See the stylesheet for the mechanism.
    <div className="keyboard-done-bar">
      <div className="keyboard-done-inner">
        <button
          type="button"
          className="keyboard-done-btn"
          // Pointer-down rather than click, with the default prevented: a tap
          // on the bar would otherwise blur the field first, unmounting this
          // button before the click could land on it.
          onPointerDown={(e) => {
            e.preventDefault();
            dismissKeyboard().catch(() => {});
            (document.activeElement as HTMLElement | null)?.blur();
          }}
        >
          Done
        </button>
      </div>
    </div>,
    document.body,
  );
}
