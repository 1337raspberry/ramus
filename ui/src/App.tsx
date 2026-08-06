import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { foregroundResync, isAuthenticated } from "./lib/commands";
import { clearGenreMetadataCache } from "./lib/genreMetadataCache";
import type { SyncProgress } from "./lib/types";
import { usePlaybackEvents } from "./lib/usePlaybackEvents";
import { useWindowTitle } from "./lib/useWindowTitle";
import { useFullscreenSync } from "./lib/useFullscreenSync";
import { useAppKeyboard } from "./lib/useAppKeyboard";
import { useIsMobile } from "./lib/useIsMobile";
import { usePlaybackStore } from "./stores/playbackStore";
import { useLibraryStore } from "./stores/libraryStore";
import { useSettingsStore } from "./stores/settingsStore";
import { useDownloadsStore } from "./stores/downloadsStore";
import { usePlaybackQualityStore } from "./stores/playbackQualityStore";
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
import GenreInfoModal from "./components/GenreInfoModal";
import GenreHoverCard from "./components/GenreHoverCard";
import OnboardingFlow, { clearOnboardingStorage } from "./components/onboarding/OnboardingFlow";
import { clearPin } from "./components/onboarding/OAuthSignIn";
import UltraBlurBackground from "./components/UltraBlurBackground";
import MobileApp from "./mobile/MobileApp";
import Toast, { useToastStore } from "./components/Toast";
import {
  applyAccent,
  DEFAULT_ACCENT,
  DEFAULT_BLUR_COLORS,
  OLED_VOID_BLUR_COLORS,
} from "./lib/accent";
import { accentFromPalette } from "./lib/vibrantColor";
import { handleAndroidBack, pushBackHandler } from "./lib/backHandler";

applyAccent(...DEFAULT_ACCENT);

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
  const backgroundStyle = useSettingsStore((s) => s.backgroundStyle);
  const blurColors = useMemo(() => {
    if (backgroundStyle === "defaultColours") return DEFAULT_BLUR_COLORS;
    if (backgroundStyle === "oledVoid") return OLED_VOID_BLUR_COLORS;
    return albumColors ?? DEFAULT_BLUR_COLORS;
  }, [albumColors, backgroundStyle]);

  // Snap accent both directions when the background style flips.
  // `defaultColours` → force brand default; anything else (dynamic OR
  // oledVoid — the void only blacks out the backdrop) → restore the
  // currently-playing album's accent from the cached vibrant palette so
  // the user doesn't have to start a new track to see the change.
  useEffect(() => {
    if (backgroundStyle === "defaultColours") {
      applyAccent(...DEFAULT_ACCENT);
    } else {
      const palette = usePlaybackStore.getState().vibrantPalette;
      if (palette) {
        const [r, g, b] = accentFromPalette(palette);
        applyAccent(r, g, b);
      }
    }
  }, [backgroundStyle]);

  useEffect(() => {
    isAuthenticated()
      .then(setAuthed)
      .catch(() => setAuthed(false));
  }, []);

  // Revert to the brand accent whenever the user is unauthenticated (e.g.
  // after sign-out). Post-auth, album-art palette extraction in the
  // playback store takes over the moment a track loads.
  useEffect(() => {
    if (authed === true) return;
    applyAccent(...DEFAULT_ACCENT);
  }, [authed]);

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
    usePlaybackQualityStore.getState().ensureListener();
  }, [authed]);

  usePlaybackEvents();
  useWindowTitle();

  // Foreground resync: when the OS resumes the app (phone unlock, app
  // switch back, laptop wake) the webview may have slept through every
  // playback/connection event that fired during an outage — and the
  // stores are pure event replay, so nothing else would ever correct
  // them. Ask the backend to re-emit the authoritative snapshot and, if
  // the app woke up offline or with playback interrupted, re-evaluate the
  // connection. Debounced: rapid hide/show flips (notification shade,
  // app-switcher peek) shouldn't hammer the IPC.
  useEffect(() => {
    if (authed !== true) return;
    let lastResync = 0;
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      const now = Date.now();
      if (now - lastResync < 3000) return;
      lastResync = now;
      foregroundResync().catch(() => {});
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [authed]);

  // App-lifetime sync listener: invalidates cached genre metadata on any
  // completion (in-library flags are baked into the responses — the settings
  // panel's own listener dies with the panel, and auto-syncs never open it),
  // and surfaces background syncs as toasts. Toasts are suppressed while the
  // settings panel is open, since its progress banner already shows the sync.
  const showSettingsRef = useRef(showSettings);
  showSettingsRef.current = showSettings;
  useEffect(() => {
    let syncActive = false;
    const unlisten = listen<SyncProgress>("sync-progress", (event) => {
      const phase = event.payload.phase;
      const toast = (msg: string) => {
        if (!showSettingsRef.current) useToastStore.getState().show(msg);
      };
      if (phase === "done") {
        clearGenreMetadataCache();
        if (syncActive) toast("Library sync finished");
        syncActive = false;
      } else if (phase === "error") {
        if (syncActive) toast("Library sync failed");
        syncActive = false;
      } else {
        if (!syncActive) toast("Syncing library…");
        syncActive = true;
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // When effective-offline flips (either manually or because the server
  // became reachable / went away), reload the library so filtered vs full
  // results take effect without the user having to navigate.
  const connection = useConnectionStatus();
  useEffect(() => {
    if (authed !== true) return;
    // Offline mode also changes the in-library flags baked into cached
    // genre metadata (they follow the downloaded-only view), so the
    // session cache must flip with the library.
    clearGenreMetadataCache();
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
        detail={
          <DetailColumn
            onOpenEQ={() => setShowEQ(true)}
            onOpenSettings={() => setShowSettings(true)}
          />
        }
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
      <GenreInfoModal />
      <GenreHoverCard />
      <Toast />
    </>
  );
}
