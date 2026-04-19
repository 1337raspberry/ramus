import type { SavedSearch } from "./types";

function newId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `ss_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Build a `SavedSearch` from input, falling back to the query as the
 * display name when `name` is blank. Generates a fresh UUID client-side.
 */
export function makeSavedSearch(query: string, name?: string): SavedSearch {
  const q = query.trim();
  const n = (name ?? "").trim();
  return { id: newId(), name: n || q, query: q };
}

/**
 * Case-insensitive uniqueness check across a saved-search list, comparing
 * against each entry's `name` field. `ignoreId` lets the row being edited
 * keep its own current name on save.
 */
export function isNameUnique(list: SavedSearch[], candidate: string, ignoreId?: string): boolean {
  const needle = candidate.trim().toLowerCase();
  return !list.some((s) => s.id !== ignoreId && s.name.trim().toLowerCase() === needle);
}
