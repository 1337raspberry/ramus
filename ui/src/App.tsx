import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isAuthenticated, togglePlayPause, nextTrack, previousTrack } from "./lib/commands";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  PlaybackBufferingPayload,
} from "./lib/types";
import { usePlaybackStore } from "./stores/playbackStore";
import { useLibraryStore } from "./stores/libraryStore";
import ThreeColumnLayout from "./components/ThreeColumnLayout";
import SidebarView from "./components/SidebarView";
import AlbumGridView from "./components/AlbumGridView";
import AlbumDetailView from "./components/AlbumDetailView";
import SuggestionView from "./components/SuggestionView";
import DetailColumn from "./components/DetailColumn";
import SearchOverlay from "./components/SearchOverlay";
import EqualizerPanel from "./components/EqualizerPanel";
import LibrarySettingsPanel from "./components/LibrarySettingsPanel";
import OnboardingFlow from "./components/onboarding/OnboardingFlow";
import UltraBlurBackground, { randomPalette } from "./components/UltraBlurBackground";
import ColorDebugPanel from "./components/ColorDebugPanel";
import { IconClose, IconMinimize, IconFullscreen } from "./components/Icons";

const appWindow = getCurrentWindow();

function TrafficLights() {
  return (
    <div className="drag-region" data-tauri-drag-region>
      <div className="traffic-lights">
        <button className="traffic-light tl-close" title="Close" onClick={() => appWindow.close()}>
          <IconClose size={10} />
        </button>
        <button className="traffic-light tl-minimize" title="Minimize" onClick={() => appWindow.minimize()}>
          <IconMinimize size={10} />
        </button>
        <button className="traffic-light tl-fullscreen" title="Fullscreen" onClick={() => appWindow.toggleMaximize()}>
          <IconFullscreen size={10} />
        </button>
      </div>
    </div>
  );
}

// Generate once at module level so it persists across re-renders
const initialPalette = randomPalette();
const initialColors = initialPalette.colors;

// Apply initial accent color immediately
document.documentElement.style.setProperty("--accent-r", String(initialPalette.accent[0]));
document.documentElement.style.setProperty("--accent-g", String(initialPalette.accent[1]));
document.documentElement.style.setProperty("--accent-b", String(initialPalette.accent[2]));

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [showEQ, setShowEQ] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showColorDebug, setShowColorDebug] = useState(false);
  const suggestion = useLibraryStore((s) => s.suggestion);
  const detailAlbum = useLibraryStore((s) => s.detailAlbum);
  const albumColors = usePlaybackStore((s) => s.ultraBlurColors);
  const blurColors = useMemo(() => albumColors ?? initialColors, [albumColors]);

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

    if (mod && e.shiftKey && e.key === "D") {
      e.preventDefault();
      setShowColorDebug((s) => !s);
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
        <UltraBlurBackground colors={blurColors} />
        <TrafficLights />
        <div className="empty-state">loading...</div>
      </>
    );
  }

  // Not authenticated — onboarding flow
  if (!authed) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        <TrafficLights />
        <OnboardingFlow onComplete={() => setAuthed(true)} />
      </>
    );
  }

  // Authenticated — main layout
  return (
    <>
      <UltraBlurBackground colors={blurColors} />
      <TrafficLights />
      <ThreeColumnLayout
        sidebar={<SidebarView onOpenSettings={() => setShowSettings(true)} />}
        content={detailAlbum ? <AlbumDetailView /> : suggestion ? <SuggestionView /> : <AlbumGridView />}
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
      {showColorDebug && <ColorDebugPanel />}
    </>
  );
}
