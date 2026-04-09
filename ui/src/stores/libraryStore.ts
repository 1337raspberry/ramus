import { create } from "zustand";
import type { Album, ArtistInfo, GenreNode, Track } from "../lib/types";
import {
  getGenreTree,
  getFavouriteGenreTree,
  getAlbumsForGenre,
  getAllAlbums,
  getFavouriteAlbums,
  getAlbumsForArtist,
  getAlbumsForArtistName,
  getAlbumsForYear,
  getTracksForAlbum,
  getAllArtists,
  getRandomAlbum,
  toggleAlbumFavourite,
  toggleTrackFavourite,
  playTracks,
} from "../lib/commands";

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
  loadGenreTree: () => Promise<void>;
  loadFavouriteGenreTree: () => Promise<void>;
  toggleGenreExpanded: (id: string) => void;
  expandAll: () => void;
  collapseAll: () => void;
  selectGenre: (node: GenreNode) => void;

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
    set({ sidebarMode: mode, suggestion: null, detailAlbum: null });
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

  loadGenreTree: async () => {
    try {
      const resp = await getGenreTree();
      set({ genreTree: resp.tree, totalAlbumCount: resp.totalAlbumCount });
    } catch {
      // Cache may not be initialized yet
    }
  },

  loadFavouriteGenreTree: async () => {
    try {
      const resp = await getFavouriteGenreTree();
      set({ genreTree: resp.tree, totalAlbumCount: resp.totalAlbumCount });
    } catch {
      // Cache may not be initialized yet
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
    set({ selectedGenreId: node.id, suggestion: null, detailAlbum: null });
    if (get().sidebarMode === "favourites") {
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
    set({ selectedArtistId: sourceId, suggestion: null, detailAlbum: null });
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
    try {
      const albums = await getAlbumsForArtistName(name);
      set((state) => ({
        albums: sortAlbums(albums, state.albumSortOrder),
        detailAlbum: null,
        suggestion: null,
      }));
    } catch {}
  },

  loadAlbumsForYear: async (year) => {
    try {
      const albums = await getAlbumsForYear(year);
      set((state) => ({
        albums: sortAlbums(albums, state.albumSortOrder),
        detailAlbum: null,
        suggestion: null,
      }));
    } catch {}
  },

  shuffleAlbums: () => set((state) => ({ albums: sortAlbums(state.albums, "random") })),

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
    } catch {}
  },

  playAlbum: async (album, startAt = 0) => {
    try {
      let { tracks } = get();
      if (!tracks.length || get().selectedAlbum?.ratingKey !== album.ratingKey) {
        tracks = await getTracksForAlbum(album.ratingKey);
        set({ selectedAlbum: album, tracks });
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
