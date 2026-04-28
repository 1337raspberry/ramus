import type { Bookmark } from "./types";
import type { AlbumFilters } from "../stores/libraryStore";

function newId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `bm_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Build a `Bookmark` from input. Generates a fresh UUID client-side; the
 * filter snapshot is provided by the caller from the live filter state.
 */
export function makeBookmark(name: string, filters: Bookmark["filters"]): Bookmark {
  return { id: newId(), name: name.trim(), filters };
}

/**
 * Case-insensitive uniqueness check across a bookmark list, comparing
 * against each entry's `name` field. `ignoreId` lets the row being edited
 * keep its own current name on save.
 */
export function isNameUnique(list: Bookmark[], candidate: string, ignoreId?: string): boolean {
  const needle = candidate.trim().toLowerCase();
  return !list.some((b) => b.id !== ignoreId && b.name.trim().toLowerCase() === needle);
}

/**
 * Project the IPC-shaped `Bookmark.filters` (with `yearMin`/`yearMax`) into
 * the client-side `AlbumFilters` shape (with the raw `year` string) so
 * `describeFilters` can present it. Min/max → "min-max" or single year if
 * equal; one-sided bounds round-trip via `>=`/`<=` because the IPC drops
 * exclusive vs inclusive distinction (`>2000` and `>=2001` collapse to the
 * same `yearMin: 2001`).
 */
export function filtersFromBookmark(b: Bookmark): AlbumFilters {
  const f = b.filters;
  let year = "";
  if (f.yearMin != null && f.yearMax != null) {
    year = f.yearMin === f.yearMax ? `${f.yearMin}` : `${f.yearMin}-${f.yearMax}`;
  } else if (f.yearMin != null) {
    year = `>=${f.yearMin}`;
  } else if (f.yearMax != null) {
    year = `<=${f.yearMax}`;
  }
  return {
    unplayed: f.unplayed,
    favouriteAlbums: f.favouriteAlbums,
    favouriteTracks: f.favouriteTracks,
    year,
    countries: f.countries,
    genres: f.genres,
    collection: f.collection ?? "",
  };
}
