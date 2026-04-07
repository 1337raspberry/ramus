import { create } from "zustand";
import type { Album, ArtistInfo, GenreNode, Track } from "../lib/types";
import {
  getGenreTree,
  getFavouriteGenreTree,
  getAlbumsForGenre,
  getAllAlbums,
  getFavouriteAlbums,
  getAlbumsForArtist,
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
  // Sidebar
  sidebarMode: SidebarMode;
  setSidebarMode: (mode: SidebarMode) => void;

  // Genre tree
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

  // Artists
  artists: ArtistInfo[];
  selectedArtistId: string | null;
  loadArtists: () => Promise<void>;
  selectArtist: (sourceId: string) => void;

  // Albums
  albums: Album[];
  albumSortOrder: AlbumSortOrder;
  setAlbumSortOrder: (order: AlbumSortOrder) => void;
  loadAlbumsForGenre: (genre: string) => Promise<void>;
  loadAllAlbums: () => Promise<void>;
  loadFavouriteAlbums: () => Promise<void>;
  loadAlbumsForArtist: (sourceId: string) => Promise<void>;
  shuffleAlbums: () => void;

  // Selected album & tracks
  selectedAlbum: Album | null;
  tracks: Track[];
  selectAlbum: (album: Album) => Promise<void>;
  clearSelectedAlbum: () => void;

  // Actions
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
  // Sidebar
  sidebarMode: "genres",
  setSidebarMode: (mode) => {
    set({ sidebarMode: mode });
    if (mode === "genres") get().loadGenreTree();
    else if (mode === "favourites") get().loadFavouriteGenreTree();
    else if (mode === "artists") get().loadArtists();
  },

  // Genre tree
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

  expandAll: () =>
    set((state) => ({ expandedGenreIds: collectAllIds(state.genreTree) })),

  collapseAll: () => set({ expandedGenreIds: new Set() }),

  selectGenre: (node) => {
    set({ selectedGenreId: node.id });
    get().loadAlbumsForGenre(node.name);
  },

  // Artists
  artists: [],
  selectedArtistId: null,

  loadArtists: async () => {
    try {
      const artists = await getAllArtists();
      set({ artists });
    } catch {
      // ignore
    }
  },

  selectArtist: (sourceId) => {
    set({ selectedArtistId: sourceId });
    get().loadAlbumsForArtist(sourceId);
  },

  // Albums
  albums: [],
  albumSortOrder: "alphabetical",

  setAlbumSortOrder: (order) => {
    set((state) => ({
      albumSortOrder: order,
      albums: sortAlbums(state.albums, order),
    }));
  },

  loadAlbumsForGenre: async (genre) => {
    try {
      const albums = await getAlbumsForGenre(genre);
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {
      // ignore
    }
  },

  loadAllAlbums: async () => {
    try {
      const albums = await getAllAlbums();
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {
      // ignore
    }
  },

  loadFavouriteAlbums: async () => {
    try {
      const albums = await getFavouriteAlbums();
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {
      // ignore
    }
  },

  loadAlbumsForArtist: async (sourceId) => {
    try {
      const albums = await getAlbumsForArtist(sourceId);
      set((state) => ({ albums: sortAlbums(albums, state.albumSortOrder) }));
    } catch {
      // ignore
    }
  },

  shuffleAlbums: () =>
    set((state) => ({ albums: sortAlbums(state.albums, "random") })),

  // Selected album & tracks
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

  // Actions
  toggleAlbumFav: async (album) => {
    const next = !album.isFavourite;
    try {
      await toggleAlbumFavourite(album.ratingKey, next);
      set((state) => ({
        albums: state.albums.map((a) =>
          a.ratingKey === album.ratingKey ? { ...a, isFavourite: next } : a
        ),
        selectedAlbum:
          state.selectedAlbum?.ratingKey === album.ratingKey
            ? { ...state.selectedAlbum, isFavourite: next }
            : state.selectedAlbum,
      }));
    } catch {
      // ignore
    }
  },

  toggleTrackFav: async (track) => {
    const next = !track.isFavourite;
    try {
      await toggleTrackFavourite(track.ratingKey, next);
      set((state) => ({
        tracks: state.tracks.map((t) =>
          t.ratingKey === track.ratingKey ? { ...t, isFavourite: next } : t
        ),
      }));
    } catch {
      // ignore
    }
  },

  playAlbum: async (album, startAt = 0) => {
    try {
      let { tracks } = get();
      if (!tracks.length || get().selectedAlbum?.ratingKey !== album.ratingKey) {
        tracks = await getTracksForAlbum(album.ratingKey);
        set({ selectedAlbum: album, tracks });
      }
      await playTracks(tracks, startAt);
    } catch {
      // ignore
    }
  },

  playRandomAlbum: async () => {
    try {
      const album = await getRandomAlbum();
      if (album) {
        await get().selectAlbum(album);
        await get().playAlbum(album);
      }
    } catch {
      // ignore
    }
  },
}));
