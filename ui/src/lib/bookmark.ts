import type { Bookmark } from "./types";

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
