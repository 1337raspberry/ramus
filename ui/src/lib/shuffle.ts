import type { Track } from "./types";

/**
 * Stratified shuffle: spreads each key-group's items at ~1/K average spacing
 * across the result. A plain Fisher–Yates is uniformly random, but uniform
 * randomness clusters — a large same-key group lands back-to-back runs often
 * enough to read as "not shuffled". Each group is shuffled internally, then
 * its members are placed at jittered evenly-spaced positions and the pool is
 * sorted by position, so a group with K members lands roughly every 1/K of
 * the list while single-member groups stay fully random.
 */
export function balancedShuffle<T>(items: T[], keyOf: (item: T) => string): T[] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    const list = groups.get(key);
    if (list) list.push(item);
    else groups.set(key, [item]);
  }

  const placed: { item: T; pos: number }[] = [];
  for (const group of groups.values()) {
    for (let i = group.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [group[i], group[j]] = [group[j], group[i]];
    }
    for (let i = 0; i < group.length; i++) {
      placed.push({ item: group[i], pos: (i + Math.random()) / group.length });
    }
  }

  placed.sort((a, b) => a.pos - b.pos);
  return placed.map((p) => p.item);
}

/** Artist-stratified track shuffle — the standard for every queue-feeding
 * shuffle affordance, so one artist's tracks spread across the queue
 * instead of clumping. */
export function shuffleTracks(tracks: Track[]): Track[] {
  return balancedShuffle(tracks, (t) => (t.trackArtist ?? t.artistName).toLowerCase());
}
