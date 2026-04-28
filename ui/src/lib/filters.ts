// Shared IPC-shaped filter type and the converter from the client-side
// `AlbumFilters` shape (raw `year` string, comma-separated history) to the
// Rust-facing structured form (numeric min/max, nullable collection).
//
// Both the IPC commands in `commands.ts` and the bookmark persistence in
// `types.ts` reference this single definition so frontend and backend stay
// in lockstep — adding a field here is a one-stop change.

import type { AlbumFilters } from "../stores/libraryStore";
import { parseYearRange } from "../stores/libraryStore";

/// Mirrors the Rust `AlbumFilterParams` struct (camelCase). The single
/// source of truth for the IPC shape; consumed by the typed IPC wrappers
/// (`getFilteredGenreTree`, `download_bookmark`, ...) and by `Bookmark`
/// for persistence.
export interface AlbumFilterParamsIPC {
  unplayed: boolean;
  favouriteAlbums: boolean;
  favouriteTracks: boolean;
  yearMin: number | null;
  yearMax: number | null;
  countries: string[];
  genres: string[];
  collection: string | null;
}

/// Translate the client-side filter shape into the IPC shape. Year string is
/// parsed into a min/max pair; an unparseable year drops to nulls. Empty
/// collection string is normalised to `null`.
export function filtersToIPC(filters: AlbumFilters): AlbumFilterParamsIPC {
  const yr = parseYearRange(filters.year);
  return {
    unplayed: filters.unplayed,
    favouriteAlbums: filters.favouriteAlbums,
    favouriteTracks: filters.favouriteTracks,
    yearMin: yr?.min ?? null,
    yearMax: yr?.max ?? null,
    countries: filters.countries,
    genres: filters.genres,
    collection: filters.collection || null,
  };
}
