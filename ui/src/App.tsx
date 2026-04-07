import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { isAuthenticated, togglePlayPause, nextTrack, previousTrack } from "./lib/commands";
import type { AccentColorPayload } from "./lib/types";
import ThreeColumnLayout from "./components/ThreeColumnLayout";
import SidebarView from "./components/SidebarView";
import AlbumGridView from "./components/AlbumGridView";
import TrackListView from "./components/TrackListView";

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null);

  // Check auth on mount
  useEffect(() => {
    isAuthenticated()
      .then(setAuthed)
      .catch(() => setAuthed(false));
  }, []);

  // Listen for accent color events
  useEffect(() => {
    const unlisten = listen<AccentColorPayload>("accent-color", (event) => {
      const { r, g, b } = event.payload;
      document.documentElement.style.setProperty("--accent-r", String(r));
      document.documentElement.style.setProperty("--accent-g", String(g));
      document.documentElement.style.setProperty("--accent-b", String(b));
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Global keyboard shortcuts
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;

    if (mod && e.key === "f") {
      e.preventDefault();
      // TODO: open search overlay (Phase 16)
      return;
    }

    if (
      e.key === " " &&
      !mod &&
      !(e.target instanceof HTMLInputElement) &&
      !(e.target instanceof HTMLTextAreaElement)
    ) {
      e.preventDefault();
      togglePlayPause();
      return;
    }

    if (mod && e.key === "ArrowRight") {
      e.preventDefault();
      nextTrack();
      return;
    }

    if (mod && e.key === "ArrowLeft") {
      e.preventDefault();
      previousTrack();
      return;
    }
  }, []);

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  // Loading state
  if (authed === null) {
    return (
      <>
        <div className="drag-region" data-tauri-drag-region />
        <div className="empty-state">loading...</div>
      </>
    );
  }

  // Not authenticated — show onboarding placeholder
  if (!authed) {
    return (
      <>
        <div className="drag-region" data-tauri-drag-region />
        <div className="empty-state">
          <div>
            <div style={{ fontSize: "2rem", fontWeight: 300, letterSpacing: "0.1em" }}>
              ramus
            </div>
            <div style={{ marginTop: "0.5rem", fontSize: "0.85rem", opacity: 0.5 }}>
              onboarding flow (Phase 17)
            </div>
          </div>
        </div>
      </>
    );
  }

  // Authenticated — main layout
  return (
    <>
      <div className="drag-region" data-tauri-drag-region />
      <ThreeColumnLayout
        sidebar={<SidebarView />}
        content={<AlbumGridView />}
        detail={<TrackListView />}
      />
    </>
  );
}
