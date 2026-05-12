import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { polyfillCountryFlagEmojis } from "country-flag-emoji-polyfill";
import App from "./App";
import flagFontUrl from "./fonts/TwemojiCountryFlags.woff2?url";
import "./styles.css";

polyfillCountryFlagEmojis("Twemoji Country Flags", flagFontUrl);

// `backdrop-filter: blur(...)` only renders convincingly on macOS WKWebView
// (Metal-accelerated). WebView2 on Windows and WebKitGTK on Linux both fall
// back to a near-transparent dark wash that's hard to read over album art.
// Tag the root so styles.css can swap `.glass` to an opaque background on
// those platforms.
const ua = navigator.userAgent;
if (ua.includes("Windows")) {
  document.documentElement.dataset.platform = "windows";
} else if (ua.includes("Linux") && !/Android|iPhone|iPad|iPod/.test(ua)) {
  document.documentElement.dataset.platform = "linux";
}

document.addEventListener("contextmenu", (e) => e.preventDefault());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
