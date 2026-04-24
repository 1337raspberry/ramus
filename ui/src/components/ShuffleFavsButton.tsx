import { useCallback, useEffect, useRef, useState } from "react";
import { getFavouriteTracks, playTracks } from "../lib/commands";
import { IconShuffle } from "./Icons";

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
      for (let i = tracks.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [tracks[i], tracks[j]] = [tracks[j], tracks[i]];
      }
      await playTracks(tracks, 0);
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
