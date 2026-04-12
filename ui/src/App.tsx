import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isAuthenticated, togglePlayPause, nextTrack, previousTrack } from "./lib/commands";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  PlaybackBufferingPayload,
  SpectrumReadyPayload,
} from "./lib/types";
import { usePlaybackStore } from "./stores/playbackStore";
import { useLibraryStore } from "./stores/libraryStore";
import { useSettingsStore } from "./stores/settingsStore";
import ThreeColumnLayout from "./components/ThreeColumnLayout";
import SidebarView from "./components/SidebarView";
import AlbumGridView from "./components/AlbumGridView";
import AlbumDetailView from "./components/AlbumDetailView";
import SuggestionView from "./components/SuggestionView";
import DetailColumn from "./components/DetailColumn";
import FocusNowPlayingView from "./components/FocusNowPlayingView";
import SearchOverlay from "./components/SearchOverlay";
import EqualizerPanel from "./components/EqualizerPanel";
import LibrarySettingsPanel from "./components/LibrarySettingsPanel";
import OnboardingFlow from "./components/onboarding/OnboardingFlow";
import UltraBlurBackground, { randomPalette } from "./components/UltraBlurBackground";
import ColorDebugPanel from "./components/ColorDebugPanel";
import { IconClose, IconMinimize, IconFullscreen, IconMaximize } from "./components/Icons";

const appWindow = getCurrentWindow();

/** true when running inside WKWebView on macOS */
const IS_MACOS = navigator.userAgent.includes("Macintosh");

/**
 * Custom window controls + drag region.
 *
 * macOS (left-aligned): close · minimize · fullscreen
 *   – Green button enters a dedicated fullscreen Space via setFullscreen.
 *   – Double-click-to-zoom is handled natively by data-tauri-drag-region
 *     (no JS handler — adding one causes a double-fire bounce).
 *   – The Rust setup hook adds NSWindowCollectionBehaviorFullScreenPrimary
 *     so setFullscreen works on a decorations:false window.
 *
 * Windows / Linux (right-aligned): minimize · maximize · close
 *   – "Maximize" button toggles maximise (not exclusive fullscreen), so
 *     the controls stay visible and the user can shrink back down.
 */
function TrafficLights() {
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

  // Windows / Linux: right-aligned, minimize → maximize → close
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
  const isFocusMode = usePlaybackStore((s) => s.isFocusMode);
  const toggleFocusMode = usePlaybackStore((s) => s.toggleFocusMode);
  const blurColors = useMemo(() => albumColors ?? initialColors, [albumColors]);

  // Check auth on mount
  useEffect(() => {
    isAuthenticated()
      .then((ok) => {
        setAuthed(ok);
        if (ok) useSettingsStore.getState().loadSettings();
      })
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
    // Focus-mode spectrum: Rust emits this when a prefetched track or
    // the current track finishes analysis. Re-pull the spectrum for the
    // currently playing track if it matches; otherwise ignore.
    const u4 = listen<SpectrumReadyPayload>("spectrum-ready", (event) => {
      store.refreshSpectrum(event.payload.ratingKey);
    });

    store.loadVolume();

    return () => {
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
      u4.then((fn) => fn());
    };
  }, []);

  // When focus mode is active, visually hide the three-column layout so the
  // focus overlay sits cleanly over the global UltraBlur background. We
  // toggle a body class (rather than conditionally rendering) so the compact
  // NowPlayingView stays mounted — its image `onLoad` handler is responsible
  // for extracting the Vibrant palette on track change, and unmounting would
  // break that flow for brand-new albums.
  useEffect(() => {
    document.body.classList.toggle("focus-mode-active", isFocusMode);
    return () => {
      document.body.classList.remove("focus-mode-active");
    };
  }, [isFocusMode]);

  // Track native fullscreen state and toggle a body class so styles can
  // hide the custom drag-region + reclaim the 32px it occupies at the top.
  // Tauri doesn't emit a dedicated fullscreen event, so we piggyback on
  // `onResized` (fires on enter/exit) and query `isFullscreen()`. macOS's
  // system menu bar still auto-reveals at the top edge, so the user can
  // exit via View > Exit Full Screen or ⌃⌘F when our chrome is hidden.
  useEffect(() => {
    let cancelled = false;
    let unlistenResize: (() => void) | null = null;

    const apply = (fs: boolean) => {
      document.body.classList.toggle("is-fullscreen", fs);
    };

    const check = async () => {
      try {
        const fs = await appWindow.isFullscreen();
        if (!cancelled) apply(fs);
      } catch {
        // ignore
      }
    };

    check();

    appWindow
      .onResized(() => check())
      .then((fn) => {
        if (cancelled) fn();
        else unlistenResize = fn;
      });

    return () => {
      cancelled = true;
      unlistenResize?.();
      document.body.classList.remove("is-fullscreen");
    };
  }, []);

  // Global keyboard shortcuts
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      // Esc exits focus mode (before any other Esc-based dismissal)
      if (e.key === "Escape" && usePlaybackStore.getState().isFocusMode) {
        e.preventDefault();
        toggleFocusMode();
        return;
      }

      // Cmd/Ctrl+Shift+N toggles focus "Now Playing" mode
      // (with Shift held, e.key is always uppercase)
      if (mod && e.shiftKey && e.key === "N") {
        e.preventDefault();
        toggleFocusMode();
        return;
      }

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
    },
    [setShowSearch, setShowEQ, setShowSettings, toggleFocusMode],
  );

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
        content={
          detailAlbum ? <AlbumDetailView /> : suggestion ? <SuggestionView /> : <AlbumGridView />
        }
        detail={<DetailColumn onOpenEQ={() => setShowEQ(true)} />}
      />
      {isFocusMode && <FocusNowPlayingView onOpenEQ={() => setShowEQ(true)} />}
      {showSearch && <SearchOverlay onDismiss={() => setShowSearch(false)} />}
      {showEQ && <EqualizerPanel onDismiss={() => setShowEQ(false)} />}
      {showSettings && (
        <LibrarySettingsPanel
          onDismiss={() => setShowSettings(false)}
          onSignOut={() => {
            setShowSettings(false);
            // Clear focus mode so the store flag doesn't outlive the
            // authenticated session — otherwise re-auth would immediately
            // drop the user back into a dangling focus overlay with no
            // track playing.
            usePlaybackStore.setState({ isFocusMode: false });
            setAuthed(false);
          }}
        />
      )}
      {showColorDebug && <ColorDebugPanel />}
    </>
  );
}
