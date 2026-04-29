import { create } from "zustand";
import type { Album, ArtistInfo, GenreNode, Track } from "../lib/types";
import { useToastStore } from "../components/Toast";
import {
  getGenreTree,
  getFilteredGenreTree,
  getAlbumsForGenre,
  getAlbumsForGenreNames,
  getAllAlbums,
  getAlbumsForArtist,
  getAlbumsForArtistName,
  getAlbumsForYear,
  getTracksForAlbum,
  getAllArtists,
  getRandomAlbum,
  getFilteredRandomAlbum,
  searchAlbumsForGrid,
  toggleAlbumFavourite,
  toggleTrackFavourite,
  playTracks,
  expandGenreToLibraryTags,
} from "../lib/commands";
import { filtersToIPC } from "../lib/filters";
import { usePlaybackStore } from "./playbackStore";
import { useConnectionStore } from "./connectionStore";
import { useDownloadsStore } from "./downloadsStore";

export type SidebarMode = "genres" | "artists";
export type AlbumSortOrder = "alphabetical" | "latestAdded" | "recentlyPlayed" | "random";

const ALBUM_SORT_ORDERS: readonly AlbumSortOrder[] = [
  "alphabetical",
  "latestAdded",
  "recentlyPlayed",
  "random",
];

function loadPersistedAlbumSort(): AlbumSortOrder {
  const stored = localStorage.getItem("ramus-album-sort");
  return ALBUM_SORT_ORDERS.includes(stored as AlbumSortOrder)
    ? (stored as AlbumSortOrder)
    : "alphabetical";
}

export interface AlbumFilters {
  unplayed: boolean;
  /** Album-level favourite — `albums.rating >= 10.0`. */
  favouriteAlbums: boolean;
  /** Track-level favourite — at least one track on the album has
   * `tracks.userRating >= 10.0`. Independent of `favouriteAlbums`; both can
   * be active and combine with AND. */
  favouriteTracks: boolean;
  year: string;
  /** OR semantics — match any selected country. */
  countries: string[];
  /** AND semantics — album must be tagged with every selected genre (each chip
   * is expanded to its subtree on the Rust side). */
  genres: string[];
  collection: string;
}

export const DEFAULT_FILTERS: AlbumFilters = {
  unplayed: false,
  favouriteAlbums: false,
  favouriteTracks: false,
  year: "",
  countries: [],
  genres: [],
  collection: "",
};

// Migrate the pre-chip shape (`country: string`, single `favourite` toggle) to
// the current shape (`countries: string[]`, split favourite booleans, `genres`
// array). Keeps existing users' filter preferences working on first load after
// each upgrade.
function migrateLegacyShape(parsed: unknown): Partial<AlbumFilters> {
  if (!parsed || typeof parsed !== "object") return {};
  const obj = parsed as Record<string, unknown>;
  const out: Partial<AlbumFilters> = {};
  if (typeof obj.unplayed === "boolean") out.unplayed = obj.unplayed;
  if (typeof obj.favouriteAlbums === "boolean") {
    out.favouriteAlbums = obj.favouriteAlbums;
  } else if (typeof obj.favourite === "boolean") {
    // Old single `favourite` toggle = album-level favourites (track-level didn't exist).
    out.favouriteAlbums = obj.favourite;
  }
  if (typeof obj.favouriteTracks === "boolean") {
    out.favouriteTracks = obj.favouriteTracks;
  }
  if (typeof obj.year === "string") out.year = obj.year;
  if (typeof obj.collection === "string") out.collection = obj.collection;
  if (Array.isArray(obj.countries)) {
    out.countries = obj.countries.filter((v): v is string => typeof v === "string");
  } else if (typeof obj.country === "string" && obj.country) {
    out.countries = [obj.country];
  }
  if (Array.isArray(obj.genres)) {
    out.genres = obj.genres.filter((v): v is string => typeof v === "string");
  }
  return out;
}

function loadPersistedFilters(): AlbumFilters {
  try {
    const raw = localStorage.getItem("ramus-album-filters");
    if (raw) return { ...DEFAULT_FILTERS, ...migrateLegacyShape(JSON.parse(raw)) };
  } catch {}
  return { ...DEFAULT_FILTERS };
}

function persistFilters(filters: AlbumFilters) {
  localStorage.setItem("ramus-album-filters", JSON.stringify(filters));
}

export function parseYearRange(input: string): { min: number | null; max: number | null } | null {
  const s = input.trim();
  if (!s) return null;

  const decade = s.match(/^(\d{3})0s$/i);
  if (decade) {
    const base = parseInt(decade[1] + "0", 10);
    return { min: base, max: base + 9 };
  }

  const range = s.match(/^(\d{4})\s*-\s*(\d{4})$/);
  if (range) return { min: parseInt(range[1], 10), max: parseInt(range[2], 10) };

  const cmp = s.match(/^([<>]=?)\s*(\d{4})$/);
  if (cmp) {
    const val = parseInt(cmp[2], 10);
    switch (cmp[1]) {
      case ">":
        return { min: val + 1, max: null };
      case ">=":
        return { min: val, max: null };
      case "<":
        return { min: null, max: val - 1 };
      case "<=":
        return { min: null, max: val };
    }
  }

  const exact = s.match(/^(\d{4})$/);
  if (exact) {
    const val = parseInt(exact[1], 10);
    return { min: val, max: val };
  }

  return null;
}

type YearPredicate = (year: number | null) => boolean;

export function parseYearFilter(input: string): YearPredicate | null {
  const yr = parseYearRange(input);
  if (!yr) return null;
  const { min, max } = yr;
  return (y) => {
    if (y == null) return false;
    if (min != null && y < min) return false;
    if (max != null && y > max) return false;
    return true;
  };
}

// Match an album's `artistCountry` against a list of selected country chips
// (OR semantics, case-insensitive, comma-tokenised because Plex sometimes
// serves multi-country artists as a comma-joined string).
function albumMatchesCountries(album: Album, countries: string[]): boolean {
  if (countries.length === 0) return true;
  if (!album.artistCountry) return false;
  const tokens = album.artistCountry
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter(Boolean);
  if (tokens.length === 0) return false;
  const wanted = new Set(countries.map((c) => c.toLowerCase()));
  return tokens.some((t) => wanted.has(t));
}

// AND semantics — album must overlap with every chip's expansion. A chip whose
// expansion hasn't been fetched yet (key absent from the map) passes through,
// otherwise we'd flash an empty grid on every new chip. Once the expansion
// lands the predicate is re-evaluated and the grid prunes correctly. An
// expansion of `[]` is a legitimate "this chip matches no library tag" and
// is correctly restrictive — distinct from "still loading".
function albumMatchesGenres(
  album: Album,
  chips: string[],
  expansions: Record<string, string[]>,
): boolean {
  if (chips.length === 0) return true;
  const albumTagsLower = album.genres.map((g) => g.toLowerCase());
  for (const chip of chips) {
    const key = chip.toLowerCase();
    if (!(key in expansions)) continue; // pending; treat as not-yet-restrictive
    const expansionSet = new Set(expansions[key]);
    if (!albumTagsLower.some((g) => expansionSet.has(g))) return false;
  }
  return true;
}

function filterAlbums(
  albums: Album[],
  filters: AlbumFilters,
  genreExpansions: Record<string, string[]>,
): Album[] {
  const yearPred = parseYearFilter(filters.year);
  return albums.filter((a) => {
    if (filters.unplayed && (a.viewCount ?? 0) > 0) return false;
    if (filters.favouriteAlbums && !a.isFavourite) return false;
    if (filters.favouriteTracks && !a.hasFavouriteTrack) return false;
    if (yearPred && !yearPred(a.year)) return false;
    if (!albumMatchesCountries(a, filters.countries)) return false;
    if (!albumMatchesGenres(a, filters.genres, genreExpansions)) return false;
    if (
      filters.collection &&
      !a.collections.some((c) => c.toLowerCase() === filters.collection.toLowerCase())
    )
      return false;
    return true;
  });
}

export function hasActiveFilters(filters: AlbumFilters): boolean {
  return (
    filters.unplayed ||
    filters.favouriteAlbums ||
    filters.favouriteTracks ||
    parseYearRange(filters.year) !== null ||
    filters.countries.length > 0 ||
    filters.genres.length > 0 ||
    filters.collection !== ""
  );
}

export function countActiveFilters(filters: AlbumFilters): number {
  return [
    filters.unplayed,
    filters.favouriteAlbums,
    filters.favouriteTracks,
    parseYearRange(filters.year) !== null,
    filters.countries.length > 0,
    filters.genres.length > 0,
    filters.collection !== "",
  ].filter(Boolean).length;
}

function sortAndFilter(
  albums: Album[],
  order: AlbumSortOrder,
  filters: AlbumFilters,
  genreExpansions: Record<string, string[]>,
): { sorted: Album[]; filtered: Album[] } {
  const sorted = sortAlbums(albums, order);
  return { sorted, filtered: filterAlbums(sorted, filters, genreExpansions) };
}

interface LibraryState {
  // --- Sidebar ---
  sidebarMode: SidebarMode;
  setSidebarMode: (mode: SidebarMode) => void;

  // --- Genre Tree ---
  genreTree: GenreNode[];
  totalAlbumCount: number;
  expandedGenreIds: Set<string>;
  selectedGenreId: string | null;
  lastSelectedGenreId: string | null;
  loadGenreTree: () => Promise<void>;
  reloadGenreTree: () => Promise<void>;
  toggleGenreExpanded: (id: string) => void;
  expandAll: () => void;
  collapseAll: () => void;
  selectGenre: (node: GenreNode) => void;
  selectGenreOnly: (node: GenreNode) => void;
  selectGenreByName: (name: string) => Promise<void>;

  // --- Artists ---
  artists: ArtistInfo[];
  selectedArtistId: string | null;
  loadArtists: () => Promise<void>;
  selectArtist: (sourceId: string) => void;

  // --- Albums ---
  albums: Album[];
  unfilteredAlbums: Album[];
  albumSortOrder: AlbumSortOrder;
  albumFilters: AlbumFilters;
  /// Per-chip cache of "lowercased library tag names this chip's subtree
  /// covers" — populated lazily by `setAlbumFilters`. Drives the AND-filter
  /// for the album grid: see `albumMatchesGenres`. Persists for the session
  /// only; not written to localStorage (the source IPC is cheap to repeat).
  genreExpansions: Record<string, string[]>;
  setAlbumSortOrder: (order: AlbumSortOrder) => void;
  setAlbumFilters: (filters: AlbumFilters) => void;
  /// Lazy-load expansions for any current genre chips that haven't been
  /// fetched yet. Call this once at boot so chips restored from localStorage
  /// get their library-tag sets without the user having to re-toggle them.
  hydrateGenreExpansions: () => void;
  loadAlbumsForGenre: (genre: string) => void;
  loadAllAlbums: () => Promise<void>;
  loadAlbumsForArtist: (sourceId: string) => Promise<void>;
  loadAlbumsForArtistName: (name: string) => Promise<void>;
  loadAlbumsForYear: (year: number) => Promise<void>;
  shuffleAlbums: () => void;

  // --- Selected Album & Tracks ---
  selectedAlbum: Album | null;
  tracks: Track[];
  selectAlbum: (album: Album) => Promise<void>;
  clearSelectedAlbum: () => void;

  // --- Suggestion ---
  suggestion: Album | null;
  loadSuggestion: () => Promise<void>;
  clearSuggestion: () => void;

  // --- Album Detail ---
  detailAlbum: Album | null;
  detailTracks: Track[];
  openAlbumDetail: (album: Album) => Promise<void>;
  closeAlbumDetail: () => void;

  // --- Browse context (from album detail clicks) ---
  browseArtistName: string | null;
  browseYear: number | null;

  // --- Search Results ---
  searchQuery: string | null;
  /// Display name of the bookmark currently driving the album grid, or
  /// `null` if the user has navigated/edited away from it. Surfaced by
  /// `BreadcrumbBar` as the grid title when set. Cleared by every nav
  /// action and by `setAlbumFilters` so any user-initiated change breaks
  /// the bookmark association.
  activeBookmarkName: string | null;
  loadSearchResults: (query: string) => Promise<void>;
  clearSearchResults: () => void;
  /// Apply a bookmark's saved filter snapshot, clearing any browse/search/
  /// detail context first and reloading the full album list under the new
  /// filter. The `name` is surfaced as the grid title until the user
  /// edits filters or navigates. Bookmarks store the filter config (not a
  /// result snapshot), so reloading reflects newly added matching albums
  /// automatically.
  loadBookmark: (filters: AlbumFilters, name: string) => Promise<void>;

  // --- Actions ---
  toggleAlbumFav: (album: Album) => Promise<void>;
  toggleTrackFav: (track: Track) => Promise<void>;
  playAlbum: (album: Album, startAt?: number) => Promise<void>;
}

function sortAlbums(albums: Album[], order: AlbumSortOrder): Album[] {
  const sorted = [...albums];
  switch (order) {
    case "alphabetical":
      sorted.sort((a, b) => a.title.localeCompare(b.title));
      break;
    case "latestAdded":
      sorted.sort((a, b) => (b.addedAt ?? 0) - (a.addedAt ?? 0));
      break;
    case "recentlyPlayed":
      sorted.sort((a, b) => (b.lastViewedAt ?? 0) - (a.lastViewedAt ?? 0));
      break;
    case "random":
      for (let i = sorted.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [sorted[i], sorted[j]] = [sorted[j], sorted[i]];
      }
      break;
  }
  return sorted;
}

/**
 * Find a genre node by display name, preferring the deepest match when a
 * name appears at multiple depths (e.g. "Dream Pop" under both Pop and
 * Rock) so breadcrumbs are as specific as possible.
 */
function findDeepestNodeByName(nodes: GenreNode[], name: string): GenreNode | null {
  const nameLower = name.toLowerCase();
  let best: GenreNode | null = null;
  let bestDepth = -1;
  const walk = (list: GenreNode[], depth: number) => {
    for (const n of list) {
      if (n.name.toLowerCase() === nameLower && depth > bestDepth) {
        best = n;
        bestDepth = depth;
      }
      if (n.children) walk(n.children, depth + 1);
    }
  };
  walk(nodes, 0);
  return best;
}

let genreFetchGen = 0;

// Module-level so the dedupe survives `set()` calls without polluting state.
// Cleared on success or failure of the IPC.
const inFlightGenreExpansions = new Set<string>();

/**
 * Lazy-load each chip's library-tag expansion. Idempotent — already-cached or
 * already-in-flight chips are no-ops. As each expansion lands, re-applies the
 * album filter so the grid prunes once the data is in hand. Used both by
 * `setAlbumFilters` (for newly added chips) and at app start (for chips
 * restored from localStorage).
 */
function ensureGenreExpansions(
  chips: string[],
  set: (fn: (s: LibraryState) => Partial<LibraryState>) => void,
  get: () => LibraryState,
) {
  for (const chip of chips) {
    const key = chip.toLowerCase();
    if (key in get().genreExpansions) continue;
    if (inFlightGenreExpansions.has(key)) continue;
    inFlightGenreExpansions.add(key);
    expandGenreToLibraryTags(chip)
      .then((tags) => {
        inFlightGenreExpansions.delete(key);
        set((state) => {
          const nextExpansions = { ...state.genreExpansions, [key]: tags };
          return {
            genreExpansions: nextExpansions,
            albums: filterAlbums(state.unfilteredAlbums, state.albumFilters, nextExpansions),
          };
        });
      })
      .catch(() => {
        inFlightGenreExpansions.delete(key);
      });
  }
}

function genreSelectionState(node: GenreNode, currentExpanded: Set<string>): Partial<LibraryState> {
  const segments = node.id.split("/");
  const ancestorIds = segments.slice(0, -1).map((_, i) => segments.slice(0, i + 1).join("/"));
  const nextExpanded = new Set(currentExpanded);
  ancestorIds.forEach((id) => nextExpanded.add(id));
  return {
    selectedGenreId: node.id,
    lastSelectedGenreId: node.id,
    expandedGenreIds: nextExpanded,
    suggestion: null,
    detailAlbum: null,
    browseArtistName: null,
    browseYear: null,
    searchQuery: null,
    activeBookmarkName: null,
  };
}

function fetchGenreAlbums(
  promise: Promise<Album[]>,
  set: (fn: (s: LibraryState) => Partial<LibraryState>) => void,
) {
  const gen = ++genreFetchGen;
  promise
    .then((albums) => {
      if (genreFetchGen !== gen) return;
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    })
    .catch(() => {});
}

function collectAllIds(nodes: GenreNode[]): Set<string> {
  const ids = new Set<string>();
  const visit = (node: GenreNode) => {
    ids.add(node.id);
    node.children?.forEach(visit);
  };
  nodes.forEach(visit);
  return ids;
}

export const useLibraryStore = create<LibraryState>((set, get) => ({
  // --- Sidebar ---
  sidebarMode: "genres",
  setSidebarMode: (mode) => {
    set({
      sidebarMode: mode,
      suggestion: null,
      detailAlbum: null,
      browseArtistName: null,
      browseYear: null,
      searchQuery: null,
      activeBookmarkName: null,
    });
    if (mode === "genres") {
      get().reloadGenreTree();
      get().loadAllAlbums();
      set({ selectedGenreId: "__all__" });
    } else if (mode === "artists") {
      get().loadArtists();
    }
  },

  // --- Genre Tree ---
  genreTree: [],
  totalAlbumCount: 0,
  expandedGenreIds: new Set(),
  selectedGenreId: null,
  lastSelectedGenreId: null,

  loadGenreTree: async () => {
    try {
      const resp = await getGenreTree();
      set({ genreTree: resp.tree, totalAlbumCount: resp.totalAlbumCount });
    } catch {
      // Cache not yet initialised
    }
  },

  reloadGenreTree: async () => {
    const filters = get().albumFilters;
    if (hasActiveFilters(filters)) {
      try {
        const resp = await getFilteredGenreTree(filtersToIPC(filters));
        set({ genreTree: resp.tree, totalAlbumCount: resp.totalAlbumCount });
      } catch {}
    } else {
      get().loadGenreTree();
    }
  },

  toggleGenreExpanded: (id) =>
    set((state) => {
      const next = new Set(state.expandedGenreIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { expandedGenreIds: next };
    }),

  expandAll: () => set((state) => ({ expandedGenreIds: collectAllIds(state.genreTree) })),

  collapseAll: () => set({ expandedGenreIds: new Set() }),

  selectGenre: (node) => {
    set((state) => genreSelectionState(node, state.expandedGenreIds));
    const collectLeafNames = (n: GenreNode): string[] => {
      if (!n.children?.length) return [n.name];
      return n.children.flatMap(collectLeafNames);
    };
    if (node.children?.length && node.id === "other") {
      fetchGenreAlbums(getAlbumsForGenreNames(collectLeafNames(node)), set);
    } else {
      fetchGenreAlbums(getAlbumsForGenre(node.name), set);
    }
  },

  selectGenreOnly: (node) => {
    set((state) => genreSelectionState(node, state.expandedGenreIds));
    fetchGenreAlbums(getAlbumsForGenreNames([node.name]), set);
  },

  selectGenreByName: async (name) => {
    set({
      detailAlbum: null,
      detailTracks: [],
      searchQuery: null,
      suggestion: null,
      browseArtistName: null,
      browseYear: null,
      activeBookmarkName: null,
    });
    // Skip loadAllAlbums() on mode switch: genre-specific albums load
    // below and we must not race. Await the tree so
    // findDeepestNodeByName has data to search.
    if (get().sidebarMode !== "genres") {
      set({ sidebarMode: "genres" });
      await get().loadGenreTree();
    }

    const node = findDeepestNodeByName(get().genreTree, name);
    if (node) {
      get().selectGenre(node);
    } else {
      set({
        selectedGenreId: name.toLowerCase(),
      });
      get().loadAlbumsForGenre(name);
    }
  },

  // --- Artists ---
  artists: [],
  selectedArtistId: null,

  loadArtists: async () => {
    try {
      const artists = await getAllArtists();
      set({ artists });
    } catch {}
  },

  selectArtist: (sourceId) => {
    set({
      selectedArtistId: sourceId,
      suggestion: null,
      detailAlbum: null,
      browseArtistName: null,
      browseYear: null,
      searchQuery: null,
      activeBookmarkName: null,
    });
    get().loadAlbumsForArtist(sourceId);
  },

  // --- Albums ---
  albums: [],
  unfilteredAlbums: [],
  albumSortOrder: loadPersistedAlbumSort(),
  albumFilters: loadPersistedFilters(),
  genreExpansions: {},

  setAlbumSortOrder: (order) => {
    localStorage.setItem("ramus-album-sort", order);
    set((state) => {
      const { sorted, filtered } = sortAndFilter(
        state.unfilteredAlbums,
        order,
        state.albumFilters,
        state.genreExpansions,
      );
      return { albumSortOrder: order, unfilteredAlbums: sorted, albums: filtered };
    });
  },

  setAlbumFilters: (filters) => {
    persistFilters(filters);
    set((state) => ({
      albumFilters: filters,
      albums: filterAlbums(state.unfilteredAlbums, filters, state.genreExpansions),
      activeBookmarkName: null,
    }));
    if (get().sidebarMode === "genres") {
      get().reloadGenreTree();
    }
    ensureGenreExpansions(filters.genres, set, get);
  },

  hydrateGenreExpansions: () => {
    ensureGenreExpansions(get().albumFilters.genres, set, get);
  },

  loadAlbumsForGenre: (genre) => {
    fetchGenreAlbums(getAlbumsForGenre(genre), set);
  },

  loadAllAlbums: async () => {
    try {
      const albums = await getAllAlbums();
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    } catch {}
  },

  loadAlbumsForArtist: async (sourceId) => {
    try {
      const albums = await getAlbumsForArtist(sourceId);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    } catch {}
  },

  loadAlbumsForArtistName: async (name) => {
    set({
      detailAlbum: null,
      detailTracks: [],
      searchQuery: null,
      suggestion: null,
      selectedArtistId: null,
      browseArtistName: name,
      browseYear: null,
      activeBookmarkName: null,
    });
    try {
      const albums = await getAlbumsForArtistName(name);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    } catch {}
  },

  loadAlbumsForYear: async (year) => {
    set({
      detailAlbum: null,
      detailTracks: [],
      searchQuery: null,
      suggestion: null,
      browseYear: year,
      browseArtistName: null,
      activeBookmarkName: null,
    });
    try {
      const albums = await getAlbumsForYear(year);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    } catch {}
  },

  shuffleAlbums: () =>
    set((state) => {
      const shuffled = sortAlbums(state.unfilteredAlbums, "random");
      return {
        unfilteredAlbums: shuffled,
        albums: filterAlbums(shuffled, state.albumFilters, state.genreExpansions),
      };
    }),

  // --- Browse context (from album detail clicks) ---
  browseArtistName: null,
  browseYear: null,

  // --- Search Results ---
  searchQuery: null,
  activeBookmarkName: null,

  loadSearchResults: async (query) => {
    try {
      const albums = await searchAlbumsForGrid(query);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
          state.genreExpansions,
        );
        return {
          unfilteredAlbums: sorted,
          albums: filtered,
          searchQuery: query,
          browseArtistName: null,
          browseYear: null,
          detailAlbum: null,
          suggestion: null,
          activeBookmarkName: null,
        };
      });
    } catch {
      set({
        unfilteredAlbums: [],
        albums: [],
        searchQuery: query,
        browseArtistName: null,
        browseYear: null,
        detailAlbum: null,
        suggestion: null,
        activeBookmarkName: null,
      });
    }
  },

  loadBookmark: async (filters, name) => {
    // Drop any browse/search/detail context so the grid shows the bookmark's
    // results unambiguously.
    set({
      searchQuery: null,
      browseArtistName: null,
      browseYear: null,
      detailAlbum: null,
      suggestion: null,
    });
    // Apply the bookmark's filter (persists, updates state, triggers genre
    // expansion fetches for any new chips). `setAlbumFilters` clears
    // `activeBookmarkName`, so we set it again immediately after — this is
    // the only path that should restore it.
    get().setAlbumFilters(filters);
    set({ activeBookmarkName: name });
    await get().loadAllAlbums();
  },

  clearSearchResults: () => {
    // Capture browse context before clearing to pick a fallback view.
    const { sidebarMode, selectedGenreId, selectedArtistId, browseArtistName, browseYear } = get();
    set({
      searchQuery: null,
      browseArtistName: null,
      browseYear: null,
      activeBookmarkName: null,
    });

    if (browseArtistName || browseYear) {
      get().loadAllAlbums();
      return;
    }

    if (sidebarMode === "artists" && selectedArtistId) {
      get().loadAlbumsForArtist(selectedArtistId);
    } else if (selectedGenreId === "__all__") {
      get().loadAllAlbums();
    } else if (selectedGenreId) {
      // Re-derive the genre name from the tree and reload.
      const findNode = (nodes: GenreNode[], id: string): GenreNode | null => {
        for (const n of nodes) {
          if (n.id === id) return n;
          const found = n.children ? findNode(n.children, id) : null;
          if (found) return found;
        }
        return null;
      };
      const node = findNode(get().genreTree, selectedGenreId);
      if (node) get().selectGenre(node);
    } else {
      set({ albums: [], unfilteredAlbums: [] });
    }
  },

  // --- Suggestion ---
  suggestion: null,

  loadSuggestion: async () => {
    try {
      const filters = get().albumFilters;
      const album = hasActiveFilters(filters)
        ? await getFilteredRandomAlbum(filtersToIPC(filters))
        : await getRandomAlbum();
      if (album) set({ suggestion: album });
    } catch {}
  },

  clearSuggestion: () => set({ suggestion: null }),

  // --- Album Detail ---
  detailAlbum: null,
  detailTracks: [],

  openAlbumDetail: async (album) => {
    set({ detailAlbum: album, suggestion: null });
    try {
      const tracks = await getTracksForAlbum(album.ratingKey);
      set({ detailTracks: tracks, selectedAlbum: album, tracks });
    } catch {
      set({ detailTracks: [] });
    }
  },

  closeAlbumDetail: () => set({ detailAlbum: null, detailTracks: [] }),

  // --- Selected Album & Tracks ---
  selectedAlbum: null,
  tracks: [],

  selectAlbum: async (album) => {
    set({ selectedAlbum: album });
    try {
      const tracks = await getTracksForAlbum(album.ratingKey);
      set({ tracks });
    } catch {
      set({ tracks: [] });
    }
  },

  clearSelectedAlbum: () => set({ selectedAlbum: null, tracks: [] }),

  // --- Actions ---
  toggleAlbumFav: async (album) => {
    const next = !album.isFavourite;
    try {
      await toggleAlbumFavourite(album.ratingKey, next);
      set((state) => {
        const nextUnfiltered = state.unfilteredAlbums.map((a) =>
          a.ratingKey === album.ratingKey ? { ...a, isFavourite: next } : a,
        );
        return {
          unfilteredAlbums: nextUnfiltered,
          albums: filterAlbums(nextUnfiltered, state.albumFilters, state.genreExpansions),
          selectedAlbum:
            state.selectedAlbum?.ratingKey === album.ratingKey
              ? { ...state.selectedAlbum, isFavourite: next }
              : state.selectedAlbum,
          detailAlbum:
            state.detailAlbum?.ratingKey === album.ratingKey
              ? { ...state.detailAlbum, isFavourite: next }
              : state.detailAlbum,
        };
      });
      // Source-of-truth sync: patch nowPlayingAlbum when its ratingKey
      // matches. Components must call this action rather than the
      // toggle_album_favourite IPC directly.
      usePlaybackStore.setState((state) => ({
        nowPlayingAlbum:
          state.nowPlayingAlbum?.ratingKey === album.ratingKey
            ? { ...state.nowPlayingAlbum, isFavourite: next }
            : state.nowPlayingAlbum,
      }));
    } catch {
      useToastStore.getState().show("Favourite update failed, try again later");
    }
  },

  toggleTrackFav: async (track) => {
    const next = !track.isFavourite;
    try {
      await toggleTrackFavourite(track.ratingKey, next);
      set((state) => {
        const nextTracks = state.tracks.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t,
        );
        const nextDetailTracks = state.detailTracks.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t,
        );

        // Re-derive `Album.hasFavouriteTrack` for the affected album so the
        // `favouriteTracks` chip filter prunes correctly when the user
        // unstars the last starred track on an album. We only need to look
        // at `selectedAlbum`/`detailAlbum` because the local `tracks` list
        // is what ratings are toggled from. Falls back to skipping the
        // patch when we can't infer the album key.
        const albumKey =
          track.albumKey ?? state.selectedAlbum?.ratingKey ?? state.detailAlbum?.ratingKey ?? null;

        let nextUnfiltered = state.unfilteredAlbums;
        let nextAlbums = state.albums;
        if (albumKey) {
          const stillHasFav = nextTracks.some(
            (t) => t.isFavourite && (t.albumKey ?? state.selectedAlbum?.ratingKey) === albumKey,
          );
          // Only patch when the album currently shows the opposite state —
          // avoids creating new array references on every track-fav toggle.
          const target = state.unfilteredAlbums.find((a) => a.ratingKey === albumKey);
          if (target && target.hasFavouriteTrack !== stillHasFav) {
            nextUnfiltered = state.unfilteredAlbums.map((a) =>
              a.ratingKey === albumKey ? { ...a, hasFavouriteTrack: stillHasFav } : a,
            );
            nextAlbums = filterAlbums(nextUnfiltered, state.albumFilters, state.genreExpansions);
          }
        }

        return {
          tracks: nextTracks,
          detailTracks: nextDetailTracks,
          unfilteredAlbums: nextUnfiltered,
          albums: nextAlbums,
        };
      });
      // Source-of-truth sync: patch currentTrack + queue when ratingKey
      // matches. Components must call this action rather than the
      // toggle_track_favourite IPC directly.
      usePlaybackStore.setState((state) => ({
        currentTrack:
          state.currentTrack?.ratingKey === track.ratingKey
            ? { ...state.currentTrack, isFavourite: next }
            : state.currentTrack,
        queue: state.queue.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t,
        ),
      }));
    } catch {
      useToastStore.getState().show("Favourite update failed, try again later");
    }
  },

  playAlbum: async (album, startAt = 0) => {
    try {
      let { tracks } = get();
      if (!tracks.length || get().selectedAlbum?.ratingKey !== album.ratingKey) {
        tracks = await getTracksForAlbum(album.ratingKey);
        set({ selectedAlbum: album, tracks });
      }

      // Offline: drop any track that isn't in the persistent download
      // set. mpv would otherwise receive null URLs for those entries and
      // silently stall. If the user clicked play on a specific (faded)
      // track, shift startAt to the nearest downloaded track at-or-after
      // the click.
      if (useConnectionStore.getState().effectiveOffline) {
        const downloaded = useDownloadsStore.getState().downloadedTrackIds;
        const playable: { track: Track; originalIdx: number }[] = tracks
          .map((t, idx) => ({ track: t, originalIdx: idx }))
          .filter(({ track }) => downloaded.has(track.ratingKey));
        if (playable.length === 0) return;
        const nextAt = playable.findIndex(({ originalIdx }) => originalIdx >= startAt);
        const newStart = nextAt < 0 ? 0 : nextAt;
        await playTracks(
          playable.map(({ track }) => track),
          newStart,
        );
        return;
      }

      await playTracks(tracks, startAt);
    } catch {}
  },
}));
