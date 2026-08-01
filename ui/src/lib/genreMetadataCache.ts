import { getGenreMetadata } from "./commands";
import type { GenreMetadata } from "./types";

// Session-long genre-metadata cache shared by every info surface (mobile
// sheet, desktop modal, hover card) so repeat lookups and simultaneous
// consumers coalesce into one IPC call per genre.
//
// Keys are lowercased names. A stored `null` means "fetched, the mapper has
// nothing for this genre" — that's a real answer and is cached. A rejected
// IPC call is NOT cached, so transient failures retry on the next open.
const cache = new Map<string, GenreMetadata | null>();
const inflight = new Map<string, Promise<GenreMetadata | null>>();
// Bumped on clear so an in-flight fetch that resolves afterwards can't
// re-populate the cache with pre-clear (stale) metadata.
let generation = 0;

/** Synchronous read: `undefined` = never fetched, `null` = fetched-and-missing. */
export function peekGenreMetadata(name: string): GenreMetadata | null | undefined {
  const key = name.toLowerCase();
  return cache.has(key) ? (cache.get(key) ?? null) : undefined;
}

/** Fetch with in-flight dedup; resolves `null` (uncached) on IPC failure. */
export function fetchGenreMetadata(name: string): Promise<GenreMetadata | null> {
  const key = name.toLowerCase();
  if (cache.has(key)) return Promise.resolve(cache.get(key) ?? null);
  const pending = inflight.get(key);
  if (pending) return pending;

  const gen = generation;
  const request = getGenreMetadata(name)
    .then((meta) => {
      if (gen === generation) cache.set(key, meta);
      return meta;
    })
    .catch(() => null)
    .finally(() => {
      inflight.delete(key);
    });
  inflight.set(key, request);
  return request;
}

/**
 * Drop everything. Must be called whenever the underlying metadata can
 * change: importing/removing a custom tree, switching genre source, and
 * after a library sync (which can flip the in-library flags baked into
 * description segments).
 */
export function clearGenreMetadataCache(): void {
  generation += 1;
  cache.clear();
  inflight.clear();
}
