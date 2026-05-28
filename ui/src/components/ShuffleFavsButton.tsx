import { useCallback, useEffect, useRef, useState } from "react";
import { getFavouriteTracks, playTracks } from "../lib/commands";
import type { Track } from "../lib/types";
import { IconShuffle } from "./Icons";

// Stratified shuffle: spreads each artist's tracks at ~1/K average spacing
// across the queue. Plain Fisher–Yates clusters same-artist runs on large lists.
function balancedShuffleByArtist(tracks: Track[]): Track[] {
  const groups = new Map<string, Track[]>();
  for (const t of tracks) {
    const key = (t.trackArtist ?? t.artistName).toLowerCase();
    const list = groups.get(key);
    if (list) list.push(t);
    else groups.set(key, [t]);
  }

  const placed: { track: Track; pos: number }[] = [];
  for (const group of groups.values()) {
    for (let i = group.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [group[i], group[j]] = [group[j], group[i]];
    }
    for (let i = 0; i < group.length; i++) {
      placed.push({ track: group[i], pos: (i + Math.random()) / group.length });
    }
  }

  placed.sort((a, b) => a.pos - b.pos);
  return placed.map((p) => p.track);
}

export default function ShuffleFavsButton() {
  const [confirming, setConfirming] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!confirming) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setConfirming(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [confirming]);

  const handleConfirm = useCallback(async () => {
    setConfirming(false);
    try {
      const tracks = await getFavouriteTracks();
      if (!tracks.length) return;
      await playTracks(balancedShuffleByArtist(tracks), 0);
    } catch {}
  }, []);

  return (
    <div className="filter-dropdown-wrap" ref={wrapRef}>
      <button
        className="filter-dropdown-btn"
        onClick={() => setConfirming((v) => !v)}
        title="Shuffle favourite tracks"
      >
        <IconShuffle size={14} />
      </button>
      {confirming && (
        <div className="shuffle-confirm-popover">
          <span>Play all favourite tracks?</span>
          <div className="shuffle-confirm-actions">
            <button className="shuffle-confirm-yes" onClick={handleConfirm}>
              Shuffle
            </button>
            <button className="shuffle-confirm-no" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
