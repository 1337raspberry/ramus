// Recently-tapped search results, shown on the empty search screen.
// Persisted to localStorage; entries carry enough display + navigation
// data to re-render and re-trigger the original tap without a live query.

export type RecentSearch =
  | { kind: "artist"; id: string; name: string; artUrl: string | null }
  | {
      kind: "album";
      id: string;
      title: string;
      artistName: string;
      artUrl: string | null;
    }
  | {
      kind: "track";
      id: string;
      title: string;
      artist: string;
      artUrl: string | null;
    }
  | { kind: "genre"; id: string; name: string };

const KEY = "ramus.searchRecents";
const MAX_RECENTS = 20;

export function loadRecents(): RecentSearch[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (e): e is RecentSearch =>
        e && typeof e === "object" && typeof e.kind === "string" && typeof e.id === "string",
    );
  } catch {
    return [];
  }
}

function save(entries: RecentSearch[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(entries.slice(0, MAX_RECENTS)));
  } catch {
    // Quota/private-mode failures just mean no recents — non-fatal.
  }
}

/** Prepend an entry, deduping by (kind, id). Returns the new list. */
export function addRecent(entry: RecentSearch): RecentSearch[] {
  const rest = loadRecents().filter((e) => !(e.kind === entry.kind && e.id === entry.id));
  const next = [entry, ...rest];
  save(next);
  return next.slice(0, MAX_RECENTS);
}

/** Remove one entry. Returns the new list. */
export function removeRecent(kind: RecentSearch["kind"], id: string): RecentSearch[] {
  const next = loadRecents().filter((e) => !(e.kind === kind && e.id === id));
  save(next);
  return next;
}
