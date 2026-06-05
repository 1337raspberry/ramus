import { create } from "zustand";

/// Drives the genre-info popover. Kept in a store (rather than local component
/// state) so the shared genre-pill component can open it from anywhere — album
/// detail, the now-playing sheet — without threading callbacks down the tree.
/// `target` is the genre a long-press opened; the sheet manages its own
/// drill-through stack from there. Session-only.
interface GenreInfoState {
  target: string | null;
  open: (genre: string) => void;
  close: () => void;
}

export const useGenreInfoStore = create<GenreInfoState>((set) => ({
  target: null,
  open: (genre) => set({ target: genre }),
  close: () => set({ target: null }),
}));
