import { useCallback, useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { getFavouriteTracks, playTracks } from "../lib/commands";
import { pushBackHandler } from "../lib/backHandler";
import SavedSearchEditor from "../components/SavedSearchEditor";
import SavedSearchPicker from "../components/SavedSearchPicker";
import type { SavedSearch } from "../lib/types";

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

function IconDice() {
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
      <rect x="3" y="3" width="18" height="18" rx="3" />
      <circle cx="8.5" cy="8.5" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="15.5" cy="8.5" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="8.5" cy="15.5" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="15.5" cy="15.5" r="1.2" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.2" fill="currentColor" stroke="none" />
    </svg>
  );
}

function IconSave() {
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
      <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8 15 3" />
    </svg>
  );
}

function IconShuffleStar() {
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
      <path d="M11 13L21 3" />
      <path d="M21 16v5h-5" />
      <path d="M15 15l6 6" />
      <path d="M4 4l3.5 3.5" />
      <path
        d="M5.5 14L6.74 17.5 10.5 17.56 7.46 19.8 8.56 23.3 5.5 21.05 2.44 23.3 3.54 19.8 0.5 17.56 4.26 17.5Z"
        fill="currentColor"
        stroke="none"
      />
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
  const savedSearches = useSettingsStore((s) => s.savedSearches);
  const [showEditor, setShowEditor] = useState(false);
  const [showPicker, setShowPicker] = useState(false);

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

  const loadSavedEntry = useCallback(
    (entry: SavedSearch) => {
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
      useLibraryStore.getState().loadSavedSearch(entry.query, entry.name);
      onSelect("savedSearch");
    },
    [onSelect],
  );

  const pick = (v: MobileView) => {
    useLibraryStore.setState({
      detailAlbum: null,
      detailTracks: [],
      suggestion: null,
      searchQuery: null,
      activeSavedSearchName: null,
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
    }
    onSelect(v);
  };

  useEffect(() => {
    if (!showEditor) return;
    return pushBackHandler(() => {
      setShowEditor(false);
      return true;
    });
  }, [showEditor]);

  useEffect(() => {
    if (!showPicker) return;
    return pushBackHandler(() => {
      setShowPicker(false);
      return true;
    });
  }, [showPicker]);

  const handleBrainTap = () => {
    if (savedSearches.length === 0) {
      setShowEditor(true);
    } else if (savedSearches.length === 1) {
      loadSavedEntry(savedSearches[0]);
    } else {
      setShowPicker(true);
    }
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
          <IconDice />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "savedSearch" ? " active" : ""}`}
          aria-label="Saved search"
          {...makeLongPress(handleBrainTap, () => setShowEditor(true))}
        >
          <IconSave />
        </button>
        <button
          className="mobile-toolbar-btn"
          aria-label="Shuffle favourite tracks"
          onClick={shuffleFavourites}
        >
          <IconShuffleStar />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "search" ? " active" : ""}`}
          aria-label="Search"
          onClick={() => pick("search")}
        >
          <IconMagnifier />
        </button>
      </nav>

      {showPicker && (
        <SavedSearchPicker
          variant="sheet"
          entries={savedSearches}
          onSelect={loadSavedEntry}
          onManage={() => setShowEditor(true)}
          onDismiss={() => setShowPicker(false)}
        />
      )}

      {showEditor && <SavedSearchEditor onDismiss={() => setShowEditor(false)} />}
    </>
  );
}
