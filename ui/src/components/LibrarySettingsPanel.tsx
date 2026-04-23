import { useCallback, useEffect, useRef, useState } from "react";
import {
  getSettings,
  updateSettings,
  getCacheStats,
  startFullSync,
  startIncrementalSync,
  startGenreSync,
  importCustomGenres,
  removeCustomGenres,
  hasCustomGenres as checkCustomGenres,
  logout,
} from "../lib/commands";
import type { Settings, CacheStats } from "../lib/types";
import { listen } from "@tauri-apps/api/event";
import { isHDR } from "../lib/hdr";
import type { SyncProgress } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import { ImageCacheRow, AudioCacheRow } from "./CacheStatsRow";
import AcknowledgementsPanel from "./AcknowledgementsPanel";
import { useIsMobile } from "../lib/useIsMobile";

interface Props {
  onDismiss: () => void;
  onSignOut: () => void;
  onOpenDownloads: () => void;
}

export default function LibrarySettingsPanel({ onDismiss, onSignOut, onOpenDownloads }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [syncing, setSyncing] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [genreWarnings, setGenreWarnings] = useState<string[]>([]);
  const [hasCustomGenres, setHasCustomGenres] = useState(false);
  const [showAcknowledgements, setShowAcknowledgements] = useState(false);
  const isMobile = useIsMobile();
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const errorTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showError = useCallback((msg: string) => {
    setError(msg);
    if (errorTimerRef.current) clearTimeout(errorTimerRef.current);
    errorTimerRef.current = setTimeout(() => setError(null), 5000);
  }, []);

  useEffect(() => {
    getSettings()
      .then(setSettings)
      .catch((e) => showError(`Failed to load settings: ${e}`));
    checkCustomGenres()
      .then(setHasCustomGenres)
      .catch(() => {});
    getCacheStats()
      .then(setStats)
      .catch((e) => showError(`Failed to load cache stats: ${e}`));
  }, [showError]);

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

  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      if (errorTimerRef.current) clearTimeout(errorTimerRef.current);
    };
  }, []);

  const save = useCallback(
    (patch: Partial<Settings>) => {
      if (!settings) return;
      const prev = settings;
      const next = { ...settings, ...patch };
      setSettings(next);

      if (debounceRef.current) clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        updateSettings(next)
          .then(() => useSettingsStore.setState(next))
          .catch((e) => {
            setSettings(prev);
            showError(`Failed to save settings: ${e}`);
          });
      }, 300);
    },
    [settings, showError],
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
      fn().catch((e) => {
        setSyncing(null);
        showError(`Sync failed: ${e}`);
      });
    },
    [showError],
  );

  const handleImportGenres = useCallback(() => {
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
          .then((warnings) => {
            if (warnings.length > 0) setGenreWarnings(warnings);
            setHasCustomGenres(true);
            // Backend has already updated genreSource; re-fetch to sync.
            return getSettings();
          })
          .then((fresh) => {
            setSettings(fresh);
            useSettingsStore.setState(fresh);
            useLibraryStore.getState().loadGenreTree();
          })
          .catch((e) => showError(`Failed to import genres: ${e}`));
      };
      reader.readAsText(file);
    };
    input.click();
  }, [showError]);

  const handleRemoveGenres = useCallback(() => {
    removeCustomGenres()
      .then(() => {
        setHasCustomGenres(false);
        // Backend has already updated genreSource; re-fetch to sync.
        return getSettings();
      })
      .then((fresh) => {
        setSettings(fresh);
        useSettingsStore.setState(fresh);
        setGenreWarnings([]);
        useLibraryStore.getState().loadGenreTree();
      })
      .catch((e) => showError(`Failed to remove genres: ${e}`));
  }, [showError]);

  const handleSignOut = useCallback(() => {
    logout()
      .then(() => onSignOut())
      .catch((e) => showError(`Sign out failed: ${e}`));
  }, [onSignOut, showError]);

  // Skip when the Acknowledgements overlay is open so Escape only
  // dismisses the topmost layer. Both panels attach to `window`; the
  // nested listener alone can't stop this one from firing first.
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showAcknowledgements) {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onDismiss, showAcknowledgements]);

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
          {error && <div className="settings-error">{error}</div>}

          <div className="settings-section-header">PLAYBACK</div>

          <label className="settings-row">
            <span>Playback Mode</span>
            <select
              className="sort-select"
              value={settings.playbackMode}
              onChange={(e) => save({ playbackMode: e.target.value as Settings["playbackMode"] })}
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
              onBlur={(e) => {
                const gb = Math.max(0.1, Math.min(50, Number(e.target.value)));
                save({ audioCacheLimitBytes: Math.round(gb * 1_073_741_824) });
              }}
            />
          </label>

          <label className="settings-row">
            <span>Image cache limit (GB)</span>
            <input
              type="number"
              className="settings-number-input"
              min={0.1}
              max={10}
              step={0.1}
              value={(settings.imageCacheLimitBytes / 1_073_741_824).toFixed(1)}
              onChange={(e) => {
                const gb = Math.max(0.1, Math.min(10, Number(e.target.value)));
                save({ imageCacheLimitBytes: Math.round(gb * 1_073_741_824) });
              }}
              onBlur={(e) => {
                const gb = Math.max(0.1, Math.min(10, Number(e.target.value)));
                save({ imageCacheLimitBytes: Math.round(gb * 1_073_741_824) });
              }}
            />
          </label>

          <ImageCacheRow />
          <AudioCacheRow />

          {!isMobile && (
            <label className="settings-row">
              <span>Disable visualiser</span>
              <input
                type="checkbox"
                checked={settings.disableSpectrum}
                onChange={(e) => save({ disableSpectrum: e.target.checked })}
              />
            </label>
          )}

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

          {!isMobile && (
            <label className="settings-row">
              <span>Library padding ({settings.libraryPadding + 8})</span>
              <input
                type="range"
                min={-8}
                max={8}
                value={settings.libraryPadding}
                onChange={(e) => save({ libraryPadding: Number(e.target.value) })}
              />
            </label>
          )}

          {!isMobile && (
            <label className="settings-row">
              <span>Track popularity</span>
              <select
                className="sort-select"
                value={settings.popularityDisplay}
                onChange={(e) =>
                  save({ popularityDisplay: e.target.value as Settings["popularityDisplay"] })
                }
              >
                <option value="off">Off</option>
                <option value="hot">Hot tracks</option>
                <option value="chart">Popularity chart</option>
              </select>
            </label>
          )}

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
              {stats.artistCount} artists, {stats.albumCount} albums, {stats.trackCount} tracks,{" "}
              {stats.genreCount} genres
            </div>
          )}

          <div className="settings-section-header">DOWNLOADS</div>

          <button className="settings-btn" onClick={onOpenDownloads}>
            Manage downloads
          </button>

          <label className="settings-row">
            <span>Work offline</span>
            <input
              type="checkbox"
              checked={settings.offlineMode}
              onChange={(e) => save({ offlineMode: e.target.checked })}
            />
          </label>

          <div className="settings-section-header">GENRES</div>

          <label className="settings-row">
            <span>Flat genres</span>
            <input
              type="checkbox"
              checked={settings.flatGenres}
              onChange={(e) => save({ flatGenres: e.target.checked })}
            />
          </label>

          <div className="settings-row">
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <button
                className={`settings-btn${settings.genreSource === "open" ? " active" : ""}`}
                onClick={() => {
                  const next = { ...settings, genreSource: "open" as const };
                  setSettings(next);
                  updateSettings(next)
                    .then(() => {
                      useSettingsStore.setState(next);
                      useLibraryStore.getState().loadGenreTree();
                    })
                    .catch((e) => showError(`Failed to switch genre source: ${e}`));
                }}
              >
                Open Source
              </button>
              <button
                className={`settings-btn${settings.genreSource === "custom" ? " active" : ""}`}
                disabled={!hasCustomGenres}
                title={!hasCustomGenres ? "Import custom genres first" : undefined}
                onClick={() => {
                  const next = { ...settings, genreSource: "custom" as const };
                  setSettings(next);
                  updateSettings(next)
                    .then(() => {
                      useSettingsStore.setState(next);
                      useLibraryStore.getState().loadGenreTree();
                    })
                    .catch((e) => showError(`Failed to switch genre source: ${e}`));
                }}
              >
                Custom
              </button>
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="settings-btn" onClick={handleImportGenres}>
                Import...
              </button>
              {hasCustomGenres && settings.genreSource === "custom" && (
                <button className="settings-btn" onClick={handleRemoveGenres}>
                  Remove
                </button>
              )}
            </div>
          </div>

          {genreWarnings.length > 0 && (
            <div className="settings-genre-warnings">
              {genreWarnings.map((w, i) => (
                <div key={i} className="settings-genre-warning">
                  {w}
                </div>
              ))}
            </div>
          )}

          <div className="settings-section-header">DISPLAY</div>

          <div className="settings-row">
            <span>Colour mode</span>
            <span className={`settings-color-mode ${isHDR ? "hdr" : "sdr"}`}>
              <span className="settings-color-mode-dot" />
              {isHDR ? "HDR" : "SDR"}
            </span>
          </div>

          <div className="settings-section-header">SECURITY</div>

          <label className="settings-row">
            <span>Refuse HTTP connections</span>
            <input
              type="checkbox"
              checked={settings.refuseHttp}
              onChange={(e) => save({ refuseHttp: e.target.checked })}
            />
          </label>

          <div className="settings-section-header">LEGAL</div>

          <button className="settings-btn" onClick={() => setShowAcknowledgements(true)}>
            Acknowledgements &amp; licenses
          </button>

          <div className="settings-section-header">ACCOUNT</div>

          <button className="settings-btn settings-signout" onClick={handleSignOut}>
            Sign Out
          </button>
        </div>
      </div>
      {showAcknowledgements && (
        <AcknowledgementsPanel onDismiss={() => setShowAcknowledgements(false)} />
      )}
    </div>
  );
}
