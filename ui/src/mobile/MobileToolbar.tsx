import { useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { getFavouriteTracks, playTracks, updateSettings } from "../lib/commands";
import SavedSearchModal from "./SavedSearchModal";

export type MobileView =
  | "genres"
  | "favourites"
  | "artists"
  | "suggestion"
  | "search"
  | "savedSearch";

interface Props {
  view: MobileView;
  onSelect: (view: MobileView) => void;
  onOpenSettings: () => void;
}

function IconList() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <line x1="8" y1="6" x2="21" y2="6" />
      <line x1="8" y1="12" x2="21" y2="12" />
      <line x1="8" y1="18" x2="21" y2="18" />
      <circle cx="4" cy="6" r="1.3" fill="currentColor" />
      <circle cx="4" cy="12" r="1.3" fill="currentColor" />
      <circle cx="4" cy="18" r="1.3" fill="currentColor" />
    </svg>
  );
}

function IconStarFill() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
    </svg>
  );
}

function IconPerson() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor">
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21c0-4.4 3.6-8 8-8s8 3.6 8 8" />
    </svg>
  );
}

function IconQuestion() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 9a3 3 0 0 1 6 0c0 1.5-1 2-2 2.5s-1 1-1 2" />
      <circle cx="12" cy="17" r="1.1" fill="currentColor" stroke="none" />
    </svg>
  );
}

function IconBrain() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24">
      <path
        transform="matrix(0.6484295845997974, 0, 0, 0.6484295845997974, 0.6398176291793314, 17.486322188449847)"
        d="M8.64-2.39C11.48-2.39 13.23-4.29 13.23-6.75C13.23-7.52 12.90-8.40 12.45-8.85C11.92-9.35 11.58-9.49 11.58-10.08C11.58-10.56 11.94-10.89 12.49-10.89C12.89-10.89 13.15-10.77 13.57-10.41C13.93-10.09 14.25-9.69 14.47-9.22C16.28-9.64 16.96-10.75 16.96-12.52C16.96-13.03 17.39-13.46 17.91-13.46C18.42-13.46 18.84-13.03 18.84-12.52C18.86-9.93 17.66-8.12 15.04-7.45C15.06-7.22 15.08-6.97 15.08-6.71C15.08-4.52 14.13-2.75 12.48-1.64C13.65-0.91 15.15-0.50 16.69-0.50C16.98-0.50 17.31-0.53 17.93-0.55C17.80-1.14 17.73-1.69 17.73-2.21C17.73-9.18 27.60-8.41 27.60-15.04C27.60-17.79 24.93-19.68 22.32-19.68C22.09-19.68 21.76-19.63 21.43-19.56C20.50-20.72 19.00-21.42 17.59-21.42C15.11-21.42 13.45-19.80 13.37-17.57C13.35-16.97 12.95-16.63 12.43-16.63C11.89-16.63 11.48-17.04 11.51-17.65C11.54-18.49 11.72-19.27 12.02-19.95C11.47-20.10 10.92-20.17 10.37-20.17C7.54-20.17 5.41-18.38 5.41-16.18C5.41-14.72 6.50-13.51 8.09-13.51C8.60-13.51 9.02-13.08 9.02-12.57C9.02-12.06 8.60-11.63 8.09-11.63C6.04-11.63 4.54-12.57 3.84-14.06C2.74-12.79 2.10-11.10 2.10-9.35C2.10-5.33 4.69-2.39 8.64-2.39ZM27.53 4.50C30.46 4.50 31.89 0.94 31.89-2.53C31.89-3.06 31.86-3.55 31.85-4.01C30.82-3.48 29.57-3.25 28.13-3.34C27.60-3.38 27.18-3.75 27.18-4.28C27.18-4.79 27.60-5.26 28.13-5.21C31.01-4.99 32.94-6.61 32.94-9.25C32.94-11.59 31.68-13.51 29.64-14.48C28.15-6.77 19.72-7.80 19.46-2.55C19.38-0.79 20.73 0.61 22.65 0.61C23.33 0.61 23.67 0.59 23.98 0.57C24.25 2.57 25.50 4.50 27.53 4.50Z"
        fill="currentColor"
      />
    </svg>
  );
}

function IconShuffle() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M16 3h5v5" />
      <path d="M4 20L21 3" />
      <path d="M21 16v5h-5" />
      <path d="M15 15l6 6" />
      <path d="M4 4l5 5" />
    </svg>
  );
}

function IconMagnifier() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="11" cy="11" r="7" />
      <line x1="16.5" y1="16.5" x2="21" y2="21" />
    </svg>
  );
}

async function shuffleFavourites() {
  try {
    const tracks = await getFavouriteTracks();
    if (!tracks.length) return;
    for (let i = tracks.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [tracks[i], tracks[j]] = [tracks[j], tracks[i]];
    }
    await playTracks(tracks, 0);
  } catch {}
}

export default function MobileToolbar({ view, onSelect, onOpenSettings }: Props) {
  const setSidebarMode = useLibraryStore((s) => s.setSidebarMode);
  const loadSuggestion = useLibraryStore((s) => s.loadSuggestion);
  const savedSearch = useSettingsStore((s) => s.savedSearch);
  const [showSavedSearchModal, setShowSavedSearchModal] = useState(false);

  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressTriggered = useRef(false);

  const makeLongPress = (onTap: () => void, onLongPressAction?: () => void) => ({
    onPointerDown: () => {
      longPressTriggered.current = false;
      longPressTimer.current = setTimeout(() => {
        longPressTriggered.current = true;
        (onLongPressAction ?? onOpenSettings)();
      }, 600);
    },
    onPointerUp: () => {
      if (longPressTimer.current) clearTimeout(longPressTimer.current);
      if (!longPressTriggered.current) onTap();
    },
    onPointerLeave: () => {
      if (longPressTimer.current) clearTimeout(longPressTimer.current);
    },
    onPointerCancel: () => {
      if (longPressTimer.current) clearTimeout(longPressTimer.current);
    },
  });

  const pick = (v: MobileView) => {
    useLibraryStore.setState({
      detailAlbum: null,
      detailTracks: [],
      suggestion: null,
      searchQuery: null,
      browseArtistName: null,
      browseYear: null,
      selectedGenreId: null,
      selectedArtistId: null,
    });
    usePlaybackStore.setState({ isFocusMode: false });

    if (v === "genres") {
      setSidebarMode("genres");
      useLibraryStore.setState({ selectedGenreId: null });
    } else if (v === "favourites") {
      setSidebarMode("favourites");
      useLibraryStore.setState({ selectedGenreId: null });
    } else if (v === "artists") {
      setSidebarMode("artists");
    } else if (v === "suggestion") {
      loadSuggestion();
    } else if (v === "search") {
      useLibraryStore.setState({ searchQuery: "" });
    } else if (v === "savedSearch") {
      const q = useSettingsStore.getState().savedSearch;
      if (q) {
        useLibraryStore.getState().loadSavedSearch(q);
      }
    }
    onSelect(v);
  };

  const handleBrainTap = () => {
    if (savedSearch) {
      pick("savedSearch");
    } else {
      setShowSavedSearchModal(true);
    }
  };

  const handleSavedSearchSave = async (query: string) => {
    const next = { ...useSettingsStore.getState(), savedSearch: query };
    useSettingsStore.setState({ savedSearch: query });
    await updateSettings(next).catch(() => {});
    setShowSavedSearchModal(false);
    pick("savedSearch");
  };

  const handleSavedSearchClear = async () => {
    const next = { ...useSettingsStore.getState(), savedSearch: null };
    useSettingsStore.setState({ savedSearch: null });
    await updateSettings(next).catch(() => {});
    setShowSavedSearchModal(false);
  };

  return (
    <>
      <nav className="mobile-toolbar" aria-label="Primary">
        <button
          className={`mobile-toolbar-btn${view === "genres" ? " active" : ""}`}
          aria-label="Genres (long-press for settings)"
          {...makeLongPress(() => pick("genres"))}
        >
          <IconList />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "favourites" ? " active" : ""}`}
          aria-label="Favourite genres"
          onClick={() => pick("favourites")}
        >
          <IconStarFill />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "artists" ? " active" : ""}`}
          aria-label="Artists"
          onClick={() => pick("artists")}
        >
          <IconPerson />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "suggestion" ? " active" : ""}`}
          aria-label="Feelin' lucky"
          onClick={() => pick("suggestion")}
        >
          <IconQuestion />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "savedSearch" ? " active" : ""}`}
          aria-label="Saved search"
          {...makeLongPress(handleBrainTap, () => setShowSavedSearchModal(true))}
        >
          <IconBrain />
        </button>
        <button
          className="mobile-toolbar-btn"
          aria-label="Shuffle favourite tracks"
          onClick={shuffleFavourites}
        >
          <IconShuffle />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "search" ? " active" : ""}`}
          aria-label="Search"
          onClick={() => pick("search")}
        >
          <IconMagnifier />
        </button>
      </nav>

      {showSavedSearchModal && (
        <SavedSearchModal
          initialQuery={savedSearch ?? ""}
          onSave={handleSavedSearchSave}
          onClear={savedSearch ? handleSavedSearchClear : undefined}
          onDismiss={() => setShowSavedSearchModal(false)}
        />
      )}
    </>
  );
}
