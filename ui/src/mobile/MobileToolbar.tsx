import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { getFavouriteTracks, playTracks } from "../lib/commands";

export type MobileView = "genres" | "favourites" | "artists" | "suggestion" | "search";

interface Props {
  view: MobileView;
  onSelect: (view: MobileView) => void;
  /**
   * Long-press on the first toolbar button opens settings. The Swift
   * reference uses this hidden gesture; we expose a discoverable
   * MobileSettingsRow at the bottom of the genre tree as well.
   */
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

function IconMoon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="currentColor">
      <path d="M21 12.79A9 9 0 1 1 11.21 3a7 7 0 0 0 9.79 9.79z" />
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

  // Long-press (>=600ms) on the first toolbar button opens settings.
  // Regular tap behaves normally.
  const makeLongPress = (onTap: () => void) => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    let triggered = false;
    return {
      onPointerDown: () => {
        triggered = false;
        timer = setTimeout(() => {
          triggered = true;
          onOpenSettings();
        }, 600);
      },
      onPointerUp: () => {
        if (timer) clearTimeout(timer);
        if (!triggered) onTap();
      },
      onPointerLeave: () => {
        if (timer) clearTimeout(timer);
      },
      onPointerCancel: () => {
        if (timer) clearTimeout(timer);
      },
    };
  };

  const pick = (v: MobileView) => {
    // Tapping the toolbar always exits album detail / suggestion / search
    // context so the tab becomes the visible view.
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
      // setSidebarMode sets selectedGenreId:"__all__" — on mobile we
      // want the genre tree, not the "All" album grid.
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

  return (
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
        className="mobile-toolbar-btn disabled"
        aria-label="Sleep timer (coming soon)"
        disabled
      >
        <IconMoon />
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
  );
}
