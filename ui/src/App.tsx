import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { isAuthenticated, togglePlayPause, nextTrack, previousTrack } from "./lib/commands";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  PlaybackBufferingPayload,
} from "./lib/types";
import { usePlaybackStore } from "./stores/playbackStore";
import ThreeColumnLayout from "./components/ThreeColumnLayout";
import SidebarView from "./components/SidebarView";
import AlbumGridView from "./components/AlbumGridView";
import DetailColumn from "./components/DetailColumn";
import SearchOverlay from "./components/SearchOverlay";
import EqualizerPanel from "./components/EqualizerPanel";
import LibrarySettingsPanel from "./components/LibrarySettingsPanel";

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [showEQ, setShowEQ] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

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

  // Listen for playback events
  useEffect(() => {
    const store = usePlaybackStore.getState();

    const u1 = listen<PlaybackStatePayload>("playback-state", (event) => {
      const { status, currentTrack, queueIndex } = event.payload;
      store.onPlaybackState(status, currentTrack, queueIndex);
    });
    const u2 = listen<PlaybackPositionPayload>("playback-position", (event) => {
      const { position, duration } = event.payload;
      store.onPlaybackPosition(position, duration);
    });
    const u3 = listen<PlaybackBufferingPayload>("playback-buffering", (event) => {
      const { isBuffering, bufferedFraction } = event.payload;
      store.onBuffering(isBuffering, bufferedFraction);
    });

    // Load initial volume
    store.loadVolume();

    return () => {
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
    };
  }, []);

  // Global keyboard shortcuts
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;

    if (mod && e.key === "f") {
      e.preventDefault();
      setShowSearch((s) => !s);
      return;
    }

    if (mod && e.key === "e") {
      e.preventDefault();
      setShowEQ((s) => !s);
      return;
    }

    if (mod && e.key === ",") {
      e.preventDefault();
      setShowSettings((s) => !s);
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
  }, [setShowSearch, setShowEQ, setShowSettings]);

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
        sidebar={<SidebarView onOpenSettings={() => setShowSettings(true)} />}
        content={<AlbumGridView />}
        detail={<DetailColumn onOpenEQ={() => setShowEQ(true)} />}
      />
      {showSearch && <SearchOverlay onDismiss={() => setShowSearch(false)} />}
      {showEQ && <EqualizerPanel onDismiss={() => setShowEQ(false)} />}
      {showSettings && (
        <LibrarySettingsPanel
          onDismiss={() => setShowSettings(false)}
          onSignOut={() => {
            setShowSettings(false);
            setAuthed(false);
          }}
        />
      )}
    </>
  );
}
