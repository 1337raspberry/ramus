import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { startFullSync } from "../../lib/commands";
import type { SyncProgress } from "../../lib/types";

interface Props {
  onComplete: () => void;
  onSkip: () => void;
}

export default function InitialSync({ onComplete, onSkip }: Props) {
  const [syncing, setSyncing] = useState(false);
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [done, setDone] = useState(false);

  useEffect(() => {
    const unlisten = listen<SyncProgress>("sync-progress", (event) => {
      setProgress(event.payload);
      if (event.payload.phase === "done") {
        setSyncing(false);
        setDone(true);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleStartSync = () => {
    setSyncing(true);
    setProgress(null);
    startFullSync().catch(() => setSyncing(false));
  };

  const fraction = progress && progress.total > 0 ? progress.current / progress.total : 0;

  const phaseLabel = progress
    ? {
        artists: "Syncing artists...",
        albums: "Syncing albums...",
        tracks: "Syncing tracks...",
        deepGenres: "Fetching genres...",
        done: "Sync complete!",
      }[progress.phase]
    : null;

  return (
    <div className="onboarding-step">
      <h2>{done ? "Ready!" : "Sync Your Library"}</h2>
      <p className="onboarding-subtitle">
        {done
          ? "Your library is synced and ready to explore."
          : "Sync your library metadata locally for instant genre browsing and search."}
      </p>

      {syncing && progress && (
        <div className="sync-progress-container">
          <div className="sync-progress-bar-bg">
            <div className="sync-progress-bar-fill" style={{ width: `${fraction * 100}%` }} />
          </div>
          <div className="sync-progress-label">{phaseLabel}</div>
          <div className="sync-progress-detail">{progress.detail}</div>
        </div>
      )}

      <div className="onboarding-actions">
        {!syncing && !done && (
          <>
            <button className="onboarding-primary-btn" onClick={handleStartSync}>
              Start Sync
            </button>
            <button className="onboarding-text-btn" onClick={onSkip}>
              Skip for now
            </button>
          </>
        )}
        {done && (
          <button className="onboarding-primary-btn" onClick={onComplete}>
            Get Started
          </button>
        )}
      </div>
    </div>
  );
}
