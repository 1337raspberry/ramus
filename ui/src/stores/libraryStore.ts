import { create } from "zustand";
import type { Album, ArtistInfo, GenreNode, Track } from "../lib/types";
import {
  getGenreTree,
  getFavouriteGenreTree,
  getAlbumsForGenre,
  getAlbumsForGenreNames,
  getAllAlbums,
  getFavouriteAlbums,
  getAlbumsForArtist,
  getAlbumsForArtistName,
  getAlbumsForYear,
  getTracksForAlbum,
  getAllArtists,
  getRandomAlbum,
  searchAlbumsForGrid,
  toggleAlbumFavourite,
  toggleTrackFavourite,
  playTracks,
} from "../lib/commands";
import { usePlaybackStore } from "./playbackStore";
import { useConnectionStore } from "./connectionStore";
import { useDownloadsStore } from "./downloadsStore";

export type SidebarMode = "genres" | "favourites" | "artists";
export type AlbumSortOrder = "alphabetical" | "latestAdded" | "recentlyPlayed" | "random";

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
  loadFavouriteGenreTree: () => Promise<void>;
  toggleGenreExpanded: (id: string) => void;
  expandAll: () => void;
  collapseAll: () => void;
  selectGenre: (node: GenreNode) => void;
  selectGenreByName: (name: string) => Promise<void>;

  // --- Artists ---
  artists: ArtistInfo[];
  selectedArtistId: string | null;
  loadArtists: () => Promise<void>;
  selectArtist: (sourceId: string) => void;

  // --- Albums ---
  albums: Album[];
  albumSortOrder: AlbumSortOrder;
  setAlbumSortOrder: (order: AlbumSortOrder) => void;
  loadAlbumsForGenre: (genre: string) => Promise<void>;
  loadAllAlbums: () => Promise<void>;
  loadFavouriteAlbums: () => Promise<void>;
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
  playRandomAlbum: () => Promise<void>;
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
      get().loadGenreTree();
      get().loadAllAlbums();
      set({ selectedGenreId: "__all__" });
    } else if (mode === "favourites") {
      get().loadFavouriteGenreTree();
      get().loadFavouriteAlbums();
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

  loadFavouriteGenreTree: async () => {
    try {
      const resp = await getFavouriteGenreTree();
      set({ genreTree: resp.tree, totalAlbumCount: resp.totalAlbumCount });
    } catch {
      // Cache not yet initialised
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
    // Expand ancestors so the selected node is visible. For id
    // "metal/black metal/blackgaze" this adds "metal" and
    // "metal/black metal". No-op when ancestors are already expanded.
    const segments = node.id.split("/");
    const ancestorIds = segments.slice(0, -1).map((_, i) => segments.slice(0, i + 1).join("/"));
    set((state) => {
      const nextExpanded = new Set(state.expandedGenreIds);
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
    });
    // Synthetic parents like "Other" aren't known to expand_genre, so
    // collect descendant leaf names and query them directly.
    const collectLeafNames = (n: GenreNode): string[] => {
      if (!n.children?.length) return [n.name];
      return n.children.flatMap(collectLeafNames);
    };
    const useDirectNames = node.children?.length && node.id === "other";
    if (useDirectNames) {
      const names = collectLeafNames(node);
      const fetch = getAlbumsForGenreNames(names);
      if (get().sidebarMode === "favourites") {
        fetch
          .then((albums) => {
            const favs = albums.filter((a) => a.isFavourite);
            set((state) => ({ albums: sortAlbums(favs, state.albumSortOrder) }));
          })
          .catch(() => {});
      } else {
        fetch
          .then((albums) => set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) })))
          .catch(() => {});
      }
    } else if (get().sidebarMode === "favourites") {
      getAlbumsForGenre(node.name)
        .then((albums) => {
          const favs = albums.filter((a) => a.isFavourite);
          set((state) => ({ albums: sortAlbums(favs, state.albumSortOrder) }));
        })
        .catch(() => {});
    } else {
      get().loadAlbumsForGenre(node.name);
    }
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
  albumSortOrder: (localStorage.getItem("ramus-album-sort") as AlbumSortOrder) || "alphabetical",

  setAlbumSortOrder: (order) => {
    localStorage.setItem("ramus-album-sort", order);
    set((state) => ({
      albumSortOrder: order,
      albums: sortAlbums(state.albums, order),
    }));
  },

  loadAlbumsForGenre: async (genre) => {
    try {
      const albums = await getAlbumsForGenre(genre);
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {}
  },

  loadAllAlbums: async () => {
    try {
      const albums = await getAllAlbums();
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {}
  },

  loadFavouriteAlbums: async () => {
    try {
      const albums = await getFavouriteAlbums();
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {}
  },

  loadAlbumsForArtist: async (sourceId) => {
    try {
      const albums = await getAlbumsForArtist(sourceId);
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {}
  },

  loadAlbumsForArtistName: async (name) => {
    set({
      detailAlbum: null,
      detailTracks: [],
      searchQuery: null,
      activeSavedSearchName: null,
      suggestion: null,
      browseArtistName: name,
      browseYear: null,
    });
    try {
      const albums = await getAlbumsForArtistName(name);
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
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
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {}
  },

  shuffleAlbums: () => set((state) => ({ albums: sortAlbums(state.albums, "random") })),

  // --- Browse context (from album detail clicks) ---
  browseArtistName: null,
  browseYear: null,

  // --- Search Results ---
  searchQuery: null,
  activeSavedSearchName: null,

  loadSearchResults: async (query) => {
    try {
      const albums = await searchAlbumsForGrid(query);
      set((state) => ({
        albums: sortAlbums(albums, state.albumSortOrder),
        searchQuery: query,
        activeSavedSearchName: null,
        browseArtistName: null,
        browseYear: null,
        detailAlbum: null,
        suggestion: null,
      }));
    } catch {
      set({
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
      set((state) => ({
        albums: sortAlbums(albums, state.albumSortOrder),
        activeSavedSearchName: name ?? query,
        browseArtistName: null,
        browseYear: null,
        detailAlbum: null,
        suggestion: null,
      }));
    } catch {
      set({
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
      if (sidebarMode === "favourites") get().loadFavouriteAlbums();
      else get().loadAllAlbums();
      return;
    }

    if (sidebarMode === "favourites") {
      get().loadFavouriteAlbums();
    } else if (sidebarMode === "artists" && selectedArtistId) {
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
      set({ albums: [] });
    }
  },

  // --- Suggestion ---
  suggestion: null,

  loadSuggestion: async () => {
    try {
      const album = await getRandomAlbum();
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
      set((state) => ({
        albums: state.albums.map((a) =>
          a.ratingKey === album.ratingKey ? { ...a, isFavourite: next } : a,
        ),
        selectedAlbum:
          state.selectedAlbum?.ratingKey === album.ratingKey
            ? { ...state.selectedAlbum, isFavourite: next }
            : state.selectedAlbum,
        detailAlbum:
          state.detailAlbum?.ratingKey === album.ratingKey
            ? { ...state.detailAlbum, isFavourite: next }
            : state.detailAlbum,
      }));
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

  playRandomAlbum: async () => {
    try {
      const album = await getRandomAlbum();
      if (album) {
        await get().selectAlbum(album);
        await get().playAlbum(album);
      }
    } catch {}
  },
}));
