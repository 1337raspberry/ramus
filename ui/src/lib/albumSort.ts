import type { Album } from "./types";

/** Sortable album-grid fields. All are already present on the fetched
 * `Album` shape, so sorting is purely a frontend reorder — no IPC. */
export type AlbumSortField =
  | "title"
  | "artist"
  | "dateAdded"
  | "datePlayed"
  | "plays"
  | "year"
  | "random";

export type SortDirection = "asc" | "desc";

export interface AlbumSort {
  field: AlbumSortField;
  direction: SortDirection;
}

/** Display metadata for the sort sheet/popover, in menu order. Each field
 * carries the direction it starts in when first selected (e.g. Date Added
 * defaults to newest-first, Title to A→Z). */
export const SORT_FIELDS: {
  field: AlbumSortField;
  label: string;
  defaultDirection: SortDirection;
}[] = [
  { field: "title", label: "Title", defaultDirection: "asc" },
  { field: "artist", label: "Album Artist", defaultDirection: "asc" },
  { field: "dateAdded", label: "Date Added", defaultDirection: "desc" },
  { field: "datePlayed", label: "Date Played", defaultDirection: "desc" },
  { field: "plays", label: "Plays", defaultDirection: "desc" },
  { field: "year", label: "Year", defaultDirection: "desc" },
  { field: "random", label: "Random", defaultDirection: "asc" },
];

export const DEFAULT_ALBUM_SORT: AlbumSort = { field: "title", direction: "asc" };

export function defaultDirectionFor(field: AlbumSortField): SortDirection {
  return SORT_FIELDS.find((f) => f.field === field)?.defaultDirection ?? "asc";
}

export function sortLabelFor(field: AlbumSortField): string {
  return SORT_FIELDS.find((f) => f.field === field)?.label ?? "Title";
}

/** Primary comparator per field, always ascending. Direction is applied to
 * this key only — tiebreaks below stay in their natural order so e.g. a
 * Z→A artist sort still lists each artist's albums oldest-first. */
function primaryCompare(field: AlbumSortField, a: Album, b: Album): number {
  switch (field) {
    case "title":
      return a.title.localeCompare(b.title);
    case "artist":
      return a.artistName.localeCompare(b.artistName);
    case "dateAdded":
      return (a.addedAt ?? 0) - (b.addedAt ?? 0);
    // Never-played albums (no timestamp) sort as oldest, so ascending reads
    // as "never/least recently played first".
    case "datePlayed":
      return (a.lastViewedAt ?? 0) - (b.lastViewedAt ?? 0);
    case "plays":
      return (a.viewCount ?? 0) - (b.viewCount ?? 0);
    case "year":
      return (a.year ?? 0) - (b.year ?? 0);
    case "random":
      return 0;
  }
}

function tiebreak(field: AlbumSortField, a: Album, b: Album): number {
  switch (field) {
    case "title":
      return a.artistName.localeCompare(b.artistName);
    case "artist":
      // Discography order within an artist: release year, then title.
      return (a.year ?? 0) - (b.year ?? 0) || a.title.localeCompare(b.title);
    default:
      return a.title.localeCompare(b.title);
  }
}

export function sortAlbums(albums: Album[], sort: AlbumSort): Album[] {
  const sorted = [...albums];
  if (sort.field === "random") {
    for (let i = sorted.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [sorted[i], sorted[j]] = [sorted[j], sorted[i]];
    }
    return sorted;
  }
  const dir = sort.direction === "desc" ? -1 : 1;
  sorted.sort((a, b) => dir * primaryCompare(sort.field, a, b) || tiebreak(sort.field, a, b));
  return sorted;
}

const STORAGE_KEY = "ramus-album-sort";

const FIELDS: readonly AlbumSortField[] = SORT_FIELDS.map((f) => f.field);

/** Pre-{field, direction} persisted values were bare mode strings. */
const LEGACY_MODES: Record<string, AlbumSort> = {
  alphabetical: { field: "title", direction: "asc" },
  latestAdded: { field: "dateAdded", direction: "desc" },
  recentlyPlayed: { field: "datePlayed", direction: "desc" },
  random: { field: "random", direction: "asc" },
};

export function loadPersistedAlbumSort(): AlbumSort {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return { ...DEFAULT_ALBUM_SORT };
  const legacy = LEGACY_MODES[raw];
  if (legacy) return { ...legacy };
  try {
    const parsed = JSON.parse(raw) as Partial<AlbumSort>;
    if (
      parsed &&
      FIELDS.includes(parsed.field as AlbumSortField) &&
      (parsed.direction === "asc" || parsed.direction === "desc")
    ) {
      return { field: parsed.field as AlbumSortField, direction: parsed.direction };
    }
  } catch {}
  return { ...DEFAULT_ALBUM_SORT };
}

export function persistAlbumSort(sort: AlbumSort): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(sort));
}
