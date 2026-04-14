import { useEffect, useMemo, useState } from "react";
import { isAuthenticated } from "./lib/commands";
import { usePlaybackEvents } from "./lib/usePlaybackEvents";
import { useFullscreenSync } from "./lib/useFullscreenSync";
import { useAppKeyboard } from "./lib/useAppKeyboard";
import { usePlaybackStore } from "./stores/playbackStore";
import { useLibraryStore } from "./stores/libraryStore";
import { useSettingsStore } from "./stores/settingsStore";
import TrafficLights from "./components/TrafficLights";
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
import BreadcrumbDebugPanel from "./components/BreadcrumbDebugPanel";

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
  const [searchInitial, setSearchInitial] = useState<string | undefined>();
  const [showEQ, setShowEQ] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showColorDebug, setShowColorDebug] = useState(false);
  const [showBreadcrumbDebug, setShowBreadcrumbDebug] = useState(false);
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

  // Tauri event subscriptions (accent-color, playback state/position, spectrum)
  usePlaybackEvents();

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

  // Track native fullscreen ↔ body class
  useFullscreenSync();

  // Global keyboard shortcuts
  useAppKeyboard({
    setShowSearch,
    setSearchInitial,
    setShowEQ,
    setShowSettings,
    setShowColorDebug,
    setShowBreadcrumbDebug,
    toggleFocusMode,
  });

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
      {showSearch && (
        <SearchOverlay
          initialQuery={searchInitial}
          onDismiss={() => {
            setShowSearch(false);
            setSearchInitial(undefined);
          }}
        />
      )}
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
      {showBreadcrumbDebug && <BreadcrumbDebugPanel />}
    </>
  );
}
