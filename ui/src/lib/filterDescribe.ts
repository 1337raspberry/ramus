import type { AlbumFilters } from "../stores/libraryStore";

/**
 * Build a short, human-readable summary of an `AlbumFilters` value. Used by
 * the bookmark save dialog, picker rows, and editor rows so the user can
 * see at a glance what each saved filter actually selects.
 *
 * Output style is a comma-joined sentence with semantic ordering: genre
 * subject first, then country, year, fav/unplayed flags, collection.
 */
export function describeFilters(filters: AlbumFilters): string {
  const parts: string[] = [];

  if (filters.genres.length > 0) {
    parts.push(filters.genres.join(" & "));
  }
  if (filters.countries.length > 0) {
    parts.push(`from ${filters.countries.join(" or ")}`);
  }
  const year = formatYear(filters.year);
  if (year) parts.push(year);
  if (filters.favouriteAlbums) parts.push("favourite albums");
  if (filters.favouriteTracks) parts.push("favourite tracks");
  if (filters.unplayed) parts.push("unplayed");
  if (filters.collection.trim()) parts.push(`in ${filters.collection.trim()}`);

  if (parts.length === 0) return "All albums";

  const first = parts[0];
  const head = first.charAt(0).toUpperCase() + first.slice(1);
  return parts.length === 1 ? head : `${head}, ${parts.slice(1).join(", ")}`;
}

function formatYear(raw: string): string | null {
  const s = raw.trim();
  if (!s) return null;
  const range = s.match(/^(\d{4})\s*-\s*(\d{4})$/);
  if (range) return `${range[1]}–${range[2]}`;
  const decade = s.match(/^(\d{3})0s$/i);
  if (decade) {
    const base = parseInt(decade[1] + "0", 10);
    return `${base}–${base + 9}`;
  }
  const cmp = s.match(/^([<>]=?)\s*(\d{4})$/);
  if (cmp) {
    const val = parseInt(cmp[2], 10);
    switch (cmp[1]) {
      case ">":
        return `after ${val}`;
      case ">=":
        return `${val} onwards`;
      case "<":
        return `before ${val}`;
      case "<=":
        return `${val} or earlier`;
    }
  }
  return s;
}
