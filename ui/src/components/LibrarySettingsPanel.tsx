import { useCallback, useEffect, useState } from "react";
import {
  getSettings,
  updateSettings,
  getCacheStats,
  startFullSync,
  startIncrementalSync,
  startGenreSync,
  importCustomGenres,
  removeCustomGenres,
  logout,
} from "../lib/commands";
import type { Settings, CacheStats } from "../lib/types";
import { listen } from "@tauri-apps/api/event";
import type { SyncProgress } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";

interface Props {
  onDismiss: () => void;
  onSignOut: () => void;
}

export default function LibrarySettingsPanel({ onDismiss, onSignOut }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [syncing, setSyncing] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);

  // Load settings and stats on mount
  useEffect(() => {
    getSettings()
      .then(setSettings)
      .catch(() => {});
    getCacheStats()
      .then(setStats)
      .catch(() => {});
  }, []);

  // Listen for sync progress
  useEffect(() => {
    const unlisten = listen<SyncProgress>("sync-progress", (event) => {
      setSyncProgress(event.payload);
      if (event.payload.phase === "done") {
        setSyncing(null);
        getCacheStats()
          .then(setStats)
          .catch(() => {});
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const save = useCallback(
    (patch: Partial<Settings>) => {
      if (!settings) return;
      const next = { ...settings, ...patch };
      setSettings(next);
      updateSettings(next).catch(() => {});
    },
    [settings]
  );

  const handleSync = useCallback(
    (type: "full" | "incremental" | "genre") => {
      setSyncing(type);
      setSyncProgress(null);
      const fn =
        type === "full"
          ? startFullSync
          : type === "incremental"
            ? startIncrementalSync
            : startGenreSync;
      fn().catch(() => setSyncing(null));
    },
    []
  );

  const handleImportGenres = useCallback(() => {
    // Create a file input to read text
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".txt,.text";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        const text = reader.result as string;
        importCustomGenres(text)
          .then(() => {
            save({ genreSource: "custom" });
            useLibraryStore.getState().loadGenreTree();
          })
          .catch(() => {});
      };
      reader.readAsText(file);
    };
    input.click();
  }, [save]);

  const handleRemoveGenres = useCallback(() => {
    removeCustomGenres()
      .then(() => {
        save({ genreSource: "open" });
        useLibraryStore.getState().loadGenreTree();
      })
      .catch(() => {});
  }, [save]);

  const handleSignOut = useCallback(() => {
    logout()
      .then(() => onSignOut())
      .catch(() => {});
  }, [onSignOut]);

  // Close on Escape
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onDismiss]);

  if (!settings) return null;

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  return (
    <div className="settings-backdrop" onClick={handleBackdropClick}>
      <div className="settings-panel glass">
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="settings-close" onClick={onDismiss}>
            x
          </button>
        </div>

        <div className="settings-body">
          {/* Playback */}
          <div className="settings-section-header">PLAYBACK</div>

          <label className="settings-row">
            <span>Playback Mode</span>
            <select
              className="sort-select"
              value={settings.playbackMode}
              onChange={(e) =>
                save({ playbackMode: e.target.value as Settings["playbackMode"] })
              }
            >
              <option value="directPlay">Direct Play</option>
              <option value="transcodeLosslessRemote">Transcode Lossless if Remote</option>
              <option value="transcodeLossless">Always Transcode Lossless</option>
            </select>
          </label>

          <label className="settings-row">
            <span>Prefetch {settings.lookaheadDepth} tracks ahead</span>
            <input
              type="range"
              min={1}
              max={20}
              value={settings.lookaheadDepth}
              onChange={(e) => save({ lookaheadDepth: Number(e.target.value) })}
            />
          </label>

          <label className="settings-row">
            <span>Show greeting messages</span>
            <input
              type="checkbox"
              checked={settings.showTaglines}
              onChange={(e) => save({ showTaglines: e.target.checked })}
            />
          </label>

          <label className="settings-row">
            <span>Audio cache limit (GB)</span>
            <input
              type="number"
              className="settings-number-input"
              min={0.1}
              max={50}
              step={0.1}
              value={(settings.audioCacheLimitBytes / 1_073_741_824).toFixed(1)}
              onChange={(e) => {
                const gb = Math.max(0.1, Math.min(50, Number(e.target.value)));
                save({ audioCacheLimitBytes: Math.round(gb * 1_073_741_824) });
              }}
            />
          </label>

          {/* Library */}
          <div className="settings-section-header">LIBRARY</div>

          <label className="settings-row">
            <span>Auto-sync interval</span>
            <select
              className="sort-select"
              value={settings.syncIntervalHours}
              onChange={(e) => save({ syncIntervalHours: Number(e.target.value) })}
            >
              <option value={0}>Off</option>
              <option value={1}>1 hour</option>
              <option value={2}>2 hours</option>
              <option value={4}>4 hours</option>
              <option value={6}>6 hours</option>
              <option value={12}>12 hours</option>
              <option value={24}>24 hours</option>
            </select>
          </label>

          <label className="settings-row">
            <span>Library padding</span>
            <input
              type="range"
              min={4}
              max={10}
              value={settings.libraryPadding}
              onChange={(e) => save({ libraryPadding: Number(e.target.value) })}
            />
          </label>

          <div className="settings-sync-buttons">
            <button
              className="settings-btn"
              disabled={syncing !== null}
              onClick={() => handleSync("incremental")}
            >
              {syncing === "incremental" ? "Syncing..." : "Quick Sync"}
            </button>
            <button
              className="settings-btn"
              disabled={syncing !== null}
              onClick={() => handleSync("genre")}
            >
              {syncing === "genre" ? "Syncing..." : "Genre Sync"}
            </button>
            <button
              className="settings-btn"
              disabled={syncing !== null}
              onClick={() => handleSync("full")}
            >
              {syncing === "full" ? "Syncing..." : "Full Sync"}
            </button>
          </div>

          {syncProgress && syncProgress.phase !== "done" && (
            <div className="settings-sync-progress">
              <div
                className="settings-progress-bar"
                style={{
                  width: `${syncProgress.total > 0 ? (syncProgress.current / syncProgress.total) * 100 : 0}%`,
                }}
              />
              <span className="settings-progress-text">{syncProgress.detail}</span>
            </div>
          )}

          {stats && stats.trackCount > 0 && (
            <div className="settings-stats">
              {stats.artistCount} artists, {stats.albumCount} albums, {stats.trackCount}{" "}
              tracks, {stats.genreCount} genres
            </div>
          )}

          {/* Genre source */}
          <div className="settings-section-header">GENRES</div>

          <div className="settings-row">
            <span>Source: {settings.genreSource === "custom" ? "Custom" : "Wikidata (CC0)"}</span>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="settings-btn" onClick={handleImportGenres}>
                Import...
              </button>
              {settings.genreSource === "custom" && (
                <button className="settings-btn" onClick={handleRemoveGenres}>
                  Remove
                </button>
              )}
            </div>
          </div>

          {/* Security */}
          <div className="settings-section-header">SECURITY</div>

          <label className="settings-row">
            <span>Refuse HTTP connections</span>
            <input
              type="checkbox"
              checked={settings.refuseHttp}
              onChange={(e) => save({ refuseHttp: e.target.checked })}
            />
          </label>

          {/* Account */}
          <div className="settings-section-header">ACCOUNT</div>

          <button
            className="settings-btn settings-signout"
            onClick={handleSignOut}
          >
            Sign Out
          </button>
        </div>
      </div>
    </div>
  );
}
