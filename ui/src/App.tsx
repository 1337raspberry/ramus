import { useEffect, useMemo, useState } from "react";
import { isAuthenticated } from "./lib/commands";
import { usePlaybackEvents } from "./lib/usePlaybackEvents";
import { useWindowTitle } from "./lib/useWindowTitle";
import { useFullscreenSync } from "./lib/useFullscreenSync";
import { useAppKeyboard } from "./lib/useAppKeyboard";
import { useIsMobile } from "./lib/useIsMobile";
import { usePlaybackStore } from "./stores/playbackStore";
import { useLibraryStore } from "./stores/libraryStore";
import { useSettingsStore } from "./stores/settingsStore";
import { useDownloadsStore } from "./stores/downloadsStore";
import { useConnectionStatus } from "./lib/useConnectionStatus";
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
import DownloadsPanel from "./components/DownloadsPanel";
import OnboardingFlow, { clearOnboardingStorage } from "./components/onboarding/OnboardingFlow";
import { clearPin } from "./components/onboarding/OAuthSignIn";
import UltraBlurBackground, { randomPalette } from "./components/UltraBlurBackground";
import MobileApp from "./mobile/MobileApp";
import Toast from "./components/Toast";
import { applyAccent } from "./lib/accent";
import { handleAndroidBack, pushBackHandler } from "./lib/backHandler";

const initialPalette = randomPalette();
const initialColors = initialPalette.colors;

applyAccent(initialPalette.accent[0], initialPalette.accent[1], initialPalette.accent[2]);

export default function App() {
  const isMobile = useIsMobile();
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchInitial, setSearchInitial] = useState<string | undefined>();
  const [showEQ, setShowEQ] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showDownloads, setShowDownloads] = useState(false);
  const suggestion = useLibraryStore((s) => s.suggestion);
  const detailAlbum = useLibraryStore((s) => s.detailAlbum);
  const albumColors = usePlaybackStore((s) => s.ultraBlurColors);
  const isFocusMode = usePlaybackStore((s) => s.isFocusMode);
  const toggleFocusMode = usePlaybackStore((s) => s.toggleFocusMode);
  const blurColors = useMemo(() => albumColors ?? initialColors, [albumColors]);

  useEffect(() => {
    isAuthenticated()
      .then(setAuthed)
      .catch(() => setAuthed(false));
  }, []);

  // Wire post-auth side-effects whenever `authed` flips to `true` — covers
  // both the resume path (token cached on disk, isAuthenticated returns
  // true on mount) and the fresh-onboarding path (OnboardingFlow's
  // onComplete sets authed). Without depending on `authed` here, a user
  // who signed in this session would skip `ensureListeners` entirely and
  // the downloads panel would never receive live progress events — only
  // the snapshot from each manual open would reach the UI.
  // ensureListeners is idempotent (`_listenersInstalled` guard), so it's
  // safe to call again on a second `authed=true` flip.
  useEffect(() => {
    if (authed !== true) return;
    useSettingsStore.getState().loadSettings();
    useDownloadsStore.getState().ensureListeners();
    useDownloadsStore.getState().refresh();
  }, [authed]);

  usePlaybackEvents();
  useWindowTitle();

  // When effective-offline flips (either manually or because the server
  // became reachable / went away), reload the library so filtered vs full
  // results take effect without the user having to navigate.
  const connection = useConnectionStatus();
  useEffect(() => {
    if (authed !== true) return;
    const lib = useLibraryStore.getState();
    lib.loadAllAlbums?.();
    lib.loadGenreTree?.();
  }, [authed, connection.effectiveOffline]);

  // Hydrate genre-chip expansions once at boot (after auth). Separate from
  // the offline-toggle effect because re-hydrating on every connection flip
  // is wasted work — `ensureGenreExpansions` is idempotent but each call
  // still re-walks the persisted chip array.
  useEffect(() => {
    if (authed !== true) return;
    useLibraryStore.getState().hydrateGenreExpansions?.();
  }, [authed]);

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
    toggleFocusMode,
  });

  useEffect(() => {
    if (!isMobile) return;
    const handler = (e: Event) => {
      if (handleAndroidBack()) e.preventDefault();
    };
    window.addEventListener("android-back-button", handler);
    return () => window.removeEventListener("android-back-button", handler);
  }, [isMobile]);

  useEffect(() => {
    if (!showDownloads) return;
    return pushBackHandler(() => {
      setShowDownloads(false);
      return true;
    });
  }, [showDownloads]);

  useEffect(() => {
    if (!showSettings) return;
    return pushBackHandler(() => {
      setShowSettings(false);
      return true;
    });
  }, [showSettings]);

  if (authed === null) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        {!isMobile && <TrafficLights />}
        <div className="empty-state">loading...</div>
      </>
    );
  }

  if (!authed) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        {!isMobile && <TrafficLights />}
        <OnboardingFlow onComplete={() => setAuthed(true)} />
      </>
    );
  }

  if (isMobile) {
    return (
      <>
        <UltraBlurBackground colors={blurColors} />
        <MobileApp onOpenSettings={() => setShowSettings(true)} />
        {showSettings && (
          <LibrarySettingsPanel
            onDismiss={() => setShowSettings(false)}
            onSignOut={() => {
              setShowSettings(false);
              usePlaybackStore.setState({ isFocusMode: false });
              clearOnboardingStorage();
              clearPin();
              setAuthed(false);
            }}
            onOpenDownloads={() => {
              setShowSettings(false);
              setShowDownloads(true);
            }}
          />
        )}
        {showDownloads && <DownloadsPanel onDismiss={() => setShowDownloads(false)} />}
        <Toast />
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
            usePlaybackStore.setState({ isFocusMode: false });
            clearOnboardingStorage();
            clearPin();
            setAuthed(false);
          }}
          onOpenDownloads={() => {
            setShowSettings(false);
            setShowDownloads(true);
          }}
        />
      )}
      {showDownloads && <DownloadsPanel onDismiss={() => setShowDownloads(false)} />}
      <Toast />
    </>
  );
}
