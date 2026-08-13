import { useRef, useState } from "react";
import { useLibraryStore, hasActiveFilters } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { IconDownload } from "../components/Icons";
import MobileFilterPanel from "./MobileFilterPanel";

export type MobileView = "genres" | "artists" | "suggestion" | "search" | "lists";

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

function IconStack() {
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
      <polygon points="12 2 22 7.5 12 13 2 7.5" />
      <polyline points="2 12.5 12 18 22 12.5" />
      <polyline points="2 17.5 12 23 22 17.5" />
    </svg>
  );
}

function IconFilterToolbar() {
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
      <polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3" />
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

export default function MobileToolbar({ view, onSelect, onOpenSettings }: Props) {
  const setSidebarMode = useLibraryStore((s) => s.setSidebarMode);
  const loadSuggestion = useLibraryStore((s) => s.loadSuggestion);
  const albumFilters = useLibraryStore((s) => s.albumFilters);
  const openDownloadsHub = useDownloadsStore((s) => s.openHub);
  const downloadsHubOpen = useDownloadsStore((s) => s.hubOpen);
  const [showFilter, setShowFilter] = useState(false);
  const filterActive = hasActiveFilters(albumFilters);

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
      suggestionMissed: false,
      searchQuery: null,
      browseArtistName: null,
      browseYear: null,
      browseCollectionName: null,
      browsePlaylist: null,
      selectedGenreId: null,
      selectedArtistId: null,
    });
    usePlaybackStore.setState({ isFocusMode: false });

    if (v === "genres") {
      setSidebarMode("genres");
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
          className={`mobile-toolbar-btn${view === "lists" ? " active" : ""}`}
          aria-label="Lists"
          onClick={() => pick("lists")}
        >
          <IconStack />
        </button>
        <button
          className={`mobile-toolbar-btn${filterActive ? " active" : ""}`}
          aria-label="Filter albums"
          onClick={() => setShowFilter(true)}
        >
          <IconFilterToolbar />
        </button>
        <button
          className={`mobile-toolbar-btn${downloadsHubOpen ? " active" : ""}`}
          aria-label="Downloads"
          onClick={openDownloadsHub}
        >
          <IconDownload size={22} />
        </button>
        <button
          className={`mobile-toolbar-btn${view === "search" ? " active" : ""}`}
          aria-label="Search"
          onClick={() => pick("search")}
        >
          <IconMagnifier />
        </button>
      </nav>

      {showFilter && <MobileFilterPanel onDismiss={() => setShowFilter(false)} />}
    </>
  );
}
