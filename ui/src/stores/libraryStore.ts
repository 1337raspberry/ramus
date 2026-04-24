import { create } from "zustand";
import type { Album, ArtistInfo, GenreNode, Track } from "../lib/types";
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
  type AlbumFilterParamsIPC,
} from "../lib/commands";
import { usePlaybackStore } from "./playbackStore";
import { useConnectionStore } from "./connectionStore";
import { useDownloadsStore } from "./downloadsStore";

export type SidebarMode = "genres" | "artists";
export type AlbumSortOrder = "alphabetical" | "latestAdded" | "recentlyPlayed" | "random";

export interface AlbumFilters {
  unplayed: boolean;
  favourite: boolean;
  year: string;
  country: string;
  collection: string;
}

export const DEFAULT_FILTERS: AlbumFilters = {
  unplayed: false,
  favourite: false,
  year: "",
  country: "",
  collection: "",
};

function loadPersistedFilters(): AlbumFilters {
  try {
    const raw = localStorage.getItem("ramus-album-filters");
    if (raw) return { ...DEFAULT_FILTERS, ...JSON.parse(raw) };
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

function filterAlbums(albums: Album[], filters: AlbumFilters): Album[] {
  const yearPred = parseYearFilter(filters.year);
  return albums.filter((a) => {
    if (filters.unplayed && (a.viewCount ?? 0) > 0) return false;
    if (filters.favourite && !a.isFavourite) return false;
    if (yearPred && !yearPred(a.year)) return false;
    if (filters.country && a.artistCountry !== filters.country) return false;
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
    filters.favourite ||
    parseYearRange(filters.year) !== null ||
    filters.country !== "" ||
    filters.collection !== ""
  );
}

export function countActiveFilters(filters: AlbumFilters): number {
  return [
    filters.unplayed,
    filters.favourite,
    parseYearRange(filters.year) !== null,
    filters.country !== "",
    filters.collection !== "",
  ].filter(Boolean).length;
}

function filtersToIPC(filters: AlbumFilters): AlbumFilterParamsIPC {
  const yr = parseYearRange(filters.year);
  return {
    unplayed: filters.unplayed,
    favourite: filters.favourite,
    yearMin: yr?.min ?? null,
    yearMax: yr?.max ?? null,
    country: filters.country || null,
    collection: filters.collection || null,
  };
}

function sortAndFilter(
  albums: Album[],
  order: AlbumSortOrder,
  filters: AlbumFilters,
): { sorted: Album[]; filtered: Album[] } {
  const sorted = sortAlbums(albums, order);
  return { sorted, filtered: filterAlbums(sorted, filters) };
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
  setAlbumSortOrder: (order: AlbumSortOrder) => void;
  setAlbumFilters: (filters: AlbumFilters) => void;
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
  /// Display name of the saved search currently driving the album grid,
  /// or `null` if the results came from live search / genre / etc. Used
  /// by the mobile saved-search header and by offline badges.
  activeSavedSearchName: string | null;
  loadSearchResults: (query: string) => Promise<void>;
  loadSavedSearch: (query: string, name?: string) => Promise<void>;
  clearSearchResults: () => void;

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
    activeSavedSearchName: null,
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
      activeSavedSearchName: null,
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
      activeSavedSearchName: null,
      suggestion: null,
      browseArtistName: null,
      browseYear: null,
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
      activeSavedSearchName: null,
    });
    get().loadAlbumsForArtist(sourceId);
  },

  // --- Albums ---
  albums: [],
  unfilteredAlbums: [],
  albumSortOrder: (localStorage.getItem("ramus-album-sort") as AlbumSortOrder) || "alphabetical",
  albumFilters: loadPersistedFilters(),

  setAlbumSortOrder: (order) => {
    localStorage.setItem("ramus-album-sort", order);
    set((state) => {
      const { sorted, filtered } = sortAndFilter(state.unfilteredAlbums, order, state.albumFilters);
      return { albumSortOrder: order, unfilteredAlbums: sorted, albums: filtered };
    });
  },

  setAlbumFilters: (filters) => {
    persistFilters(filters);
    set((state) => ({
      albumFilters: filters,
      albums: filterAlbums(state.unfilteredAlbums, filters),
    }));
    if (get().sidebarMode === "genres") {
      get().reloadGenreTree();
    }
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
      activeSavedSearchName: null,
      suggestion: null,
      selectedArtistId: null,
      browseArtistName: name,
      browseYear: null,
    });
    try {
      const albums = await getAlbumsForArtistName(name);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
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
      activeSavedSearchName: null,
      suggestion: null,
      browseYear: year,
      browseArtistName: null,
    });
    try {
      const albums = await getAlbumsForYear(year);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
        );
        return { unfilteredAlbums: sorted, albums: filtered };
      });
    } catch {}
  },

  shuffleAlbums: () =>
    set((state) => {
      const shuffled = sortAlbums(state.unfilteredAlbums, "random");
      return { unfilteredAlbums: shuffled, albums: filterAlbums(shuffled, state.albumFilters) };
    }),

  // --- Browse context (from album detail clicks) ---
  browseArtistName: null,
  browseYear: null,

  // --- Search Results ---
  searchQuery: null,
  activeSavedSearchName: null,

  loadSearchResults: async (query) => {
    try {
      const albums = await searchAlbumsForGrid(query);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
        );
        return {
          unfilteredAlbums: sorted,
          albums: filtered,
          searchQuery: query,
          activeSavedSearchName: null,
          browseArtistName: null,
          browseYear: null,
          detailAlbum: null,
          suggestion: null,
        };
      });
    } catch {
      set({
        unfilteredAlbums: [],
        albums: [],
        searchQuery: query,
        activeSavedSearchName: null,
        browseArtistName: null,
        browseYear: null,
        detailAlbum: null,
        suggestion: null,
      });
    }
  },

  loadSavedSearch: async (query, name) => {
    try {
      const albums = await searchAlbumsForGrid(query);
      set((state) => {
        const { sorted, filtered } = sortAndFilter(
          albums,
          state.albumSortOrder,
          state.albumFilters,
        );
        return {
          unfilteredAlbums: sorted,
          albums: filtered,
          activeSavedSearchName: name ?? query,
          browseArtistName: null,
          browseYear: null,
          detailAlbum: null,
          suggestion: null,
        };
      });
    } catch {
      set({
        unfilteredAlbums: [],
        albums: [],
        activeSavedSearchName: name ?? query,
        browseArtistName: null,
        browseYear: null,
        detailAlbum: null,
        suggestion: null,
      });
    }
  },

  clearSearchResults: () => {
    // Capture browse context before clearing to pick a fallback view.
    const { sidebarMode, selectedGenreId, selectedArtistId, browseArtistName, browseYear } = get();
    set({
      searchQuery: null,
      activeSavedSearchName: null,
      browseArtistName: null,
      browseYear: null,
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
          albums: filterAlbums(nextUnfiltered, state.albumFilters),
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
    } catch {}
  },

  toggleTrackFav: async (track) => {
    const next = !track.isFavourite;
    try {
      await toggleTrackFavourite(track.ratingKey, next);
      set((state) => ({
        tracks: state.tracks.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t,
        ),
        detailTracks: state.detailTracks.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t,
        ),
      }));
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
    } catch {}
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
