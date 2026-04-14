import { useEffect, useMemo, useState } from "react";
import { isAuthenticated } from "./lib/commands";
import { usePlaybackEvents } from "./lib/usePlaybackEvents";
import { useWindowTitle } from "./lib/useWindowTitle";
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
import BreadcrumbDebugPanel from "./components/BreadcrumbDebugPanel";

const initialPalette = randomPalette();
const initialColors = initialPalette.colors;

document.documentElement.style.setProperty("--accent-r", String(initialPalette.accent[0]));
document.documentElement.style.setProperty("--accent-g", String(initialPalette.accent[1]));
document.documentElement.style.setProperty("--accent-b", String(initialPalette.accent[2]));

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchInitial, setSearchInitial] = useState<string | undefined>();
  const [showEQ, setShowEQ] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showBreadcrumbDebug, setShowBreadcrumbDebug] = useState(false);
  const suggestion = useLibraryStore((s) => s.suggestion);
  const detailAlbum = useLibraryStore((s) => s.detailAlbum);
  const albumColors = usePlaybackStore((s) => s.ultraBlurColors);
  const isFocusMode = usePlaybackStore((s) => s.isFocusMode);
  const toggleFocusMode = usePlaybackStore((s) => s.toggleFocusMode);
  const blurColors = useMemo(() => albumColors ?? initialColors, [albumColors]);

  useEffect(() => {
    isAuthenticated()
      .then((ok) => {
        setAuthed(ok);
        if (ok) useSettingsStore.getState().loadSettings();
      })
      .catch(() => setAuthed(false));
  }, []);

  usePlaybackEvents();
  useWindowTitle();

  // Toggle a body class rather than conditionally rendering: the compact
  // NowPlayingView must stay mounted because its image onLoad handler extracts
  // the Vibrant palette on track change.
  useEffect(() => {
    document.body.classList.toggle("focus-mode-active", isFocusMode);
    return () => {
      document.body.classList.remove("focus-mode-active");
    };
  }, [isFocusMode]);

  useFullscreenSync();

  useAppKeyboard({
    setShowSearch,
    setSearchInitial,
    setShowEQ,
    setShowSettings,
    setShowBreadcrumbDebug,
    toggleFocusMode,
  });

  if (authed === null) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        <TrafficLights />
        <div className="empty-state">loading...</div>
      </>
    );
  }

  if (!authed) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        <TrafficLights />
        <OnboardingFlow onComplete={() => setAuthed(true)} />
      </>
    );
  }

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
            // Clear focus mode so re-auth doesn't drop into a dangling focus
            // overlay with no track playing.
            usePlaybackStore.setState({ isFocusMode: false });
            setAuthed(false);
          }}
        />
      )}
      {showBreadcrumbDebug && <BreadcrumbDebugPanel />}
    </>
  );
}
