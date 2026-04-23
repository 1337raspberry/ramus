import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { polyfillCountryFlagEmojis } from "country-flag-emoji-polyfill";
import App from "./App";
import flagFontUrl from "./fonts/TwemojiCountryFlags.woff2?url";
import "./styles.css";

polyfillCountryFlagEmojis("Twemoji Country Flags", flagFontUrl);

document.addEventListener("contextmenu", (e) => e.preventDefault());

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
