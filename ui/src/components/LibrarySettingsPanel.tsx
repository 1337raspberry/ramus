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
import type { Settings, CacheStats, PlaybackMode } from "../lib/types";
import { listen } from "@tauri-apps/api/event";
import type { SyncProgress } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import { ImageCacheRow, AudioCacheRow } from "./CacheStatsRow";
import AcknowledgementsPanel from "./AcknowledgementsPanel";
import { useIsMobile } from "../lib/useIsMobile";
import { TabBar } from "./TabBar";
import { HelperText } from "./HelperText";

interface Props {
  onDismiss: () => void;
  onSignOut: () => void;
  onOpenDownloads: () => void;
}

type TabId = "playback" | "library" | "storage" | "network" | "about";

// Per-mode prose shown under the dropdown so users can tell at a glance
// what each option actually does. The "Cellular" / "RemoteOrCellular"
// variants are hidden on desktop via `useIsMobile`, but the prose is here
// for completeness in case a desktop user has somehow ended up with them
// selected (e.g. settings copied from mobile).
const MODE_PROSE: Record<PlaybackMode, string> = {
  never:
    "Always stream lossless files in their original quality. Best if you're almost always on a fast or local connection.",
  cellular:
    "Transcode lossless files only when you're on cellular data. Saves your data plan without sacrificing quality at home, or helps with bad cellular coverage/speeds.",
  remote:
    "Transcode lossless files when you're not on the same local network as your server. Helps when streaming over the internet if your server has a poor upload speed.",
  remoteOrCellular:
    "Transcode lossless files if you're not on the same local network as your server, or if you're using cellular data. Ideal if the priority is stability over lossless streaming quality.",
  always: "Always transcode lossless files to the chosen bitrate.",
};

export default function LibrarySettingsPanel({ onDismiss, onSignOut, onOpenDownloads }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [syncing, setSyncing] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [genreWarnings, setGenreWarnings] = useState<string[]>([]);
  const [hasCustomGenres, setHasCustomGenres] = useState(false);
  const [showAcknowledgements, setShowAcknowledgements] = useState(false);
  const [showSpectrumConfirm, setShowSpectrumConfirm] = useState(false);
  const [activeTab, setActiveTab] = useState<TabId>("playback");
  const isMobile = useIsMobile();
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const errorTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const syncBannerTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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
        // Auto-clear the banner shortly after the "done" frame lands so
        // the user sees the completion state then it fades. Stored in
        // a ref so a follow-up sync (or unmount) cancels the pending
        // clear and doesn't wipe the new sync's progress mid-flight.
        if (syncBannerTimerRef.current) clearTimeout(syncBannerTimerRef.current);
        syncBannerTimerRef.current = setTimeout(() => {
          setSyncProgress(null);
          syncBannerTimerRef.current = null;
        }, 2000);
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
      if (syncBannerTimerRef.current) clearTimeout(syncBannerTimerRef.current);
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
      // Cancel a pending banner-clear from a just-finished sync so it
      // doesn't wipe the new sync's progress 2 s in.
      if (syncBannerTimerRef.current) {
        clearTimeout(syncBannerTimerRef.current);
        syncBannerTimerRef.current = null;
      }
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

  // Switch the genre source immediately rather than via the 300 ms
  // debounce, because loadGenreTree() needs the new source to already
  // be persisted. Cancels any pending debounced save first so a stale
  // settings snapshot can't flush after this and revert genreSource.
  const switchGenreSource = useCallback(
    (source: "open" | "custom") => {
      if (!settings) return;
      if (debounceRef.current) {
        clearTimeout(debounceRef.current);
        debounceRef.current = null;
      }
      const next = { ...settings, genreSource: source };
      setSettings(next);
      updateSettings(next)
        .then(() => {
          useSettingsStore.setState(next);
          useLibraryStore.getState().loadGenreTree();
        })
        .catch((e) => showError(`Failed to switch genre source: ${e}`));
    },
    [settings, showError],
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

  // Skip when a nested overlay (acknowledgements or confirm dialog) is open
  // so Escape only dismisses the topmost layer. Both panels attach to
  // `window`; the nested listener alone can't stop this one from firing first.
  // Also skip while settings is still loading — the panel renders nothing
  // visible until then, so dismissing on Escape would feel like the open
  // gesture was a no-op.
  const settingsLoaded = settings != null;
  useEffect(() => {
    if (!settingsLoaded) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showAcknowledgements && !showSpectrumConfirm) {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onDismiss, showAcknowledgements, showSpectrumConfirm, settingsLoaded]);

  if (!settings) return null;

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  const tabs: ReadonlyArray<{ id: TabId; label: string }> = [
    { id: "playback", label: "Playback" },
    { id: "library", label: "Library" },
    { id: "storage", label: "Storage" },
    { id: "network", label: "Network" },
    { id: "about", label: "About" },
  ];

  return (
    <div className="settings-backdrop" onClick={handleBackdropClick}>
      <div className="settings-panel glass">
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="settings-close" onClick={onDismiss}>
            x
          </button>
        </div>

        {/* Sync banner sits above the tab bar so it stays visible
            regardless of which tab the user moves to mid-sync. */}
        {syncProgress && <SyncProgressBanner progress={syncProgress} />}

        <TabBar
          tabs={tabs}
          active={activeTab}
          onChange={setActiveTab}
          ariaLabel="Settings sections"
          panelId="settings-panel-content"
        />

        {/* aria-label rather than aria-labelledby because TabBar generates
            its own internal id prefix via useId(); referencing the active
            tab button by id from outside would dangle. */}
        <div
          className="settings-body"
          role="tabpanel"
          id="settings-panel-content"
          aria-label={`${activeTab} settings`}
        >
          {error && <div className="settings-error">{error}</div>}

          {activeTab === "playback" && (
            <>
              <div className="settings-row settings-row-multi">
                <span>Transcode mode</span>
                <div className="settings-controls-pair">
                  <select
                    className="sort-select"
                    value={settings.playbackMode}
                    onChange={(e) => save({ playbackMode: e.target.value as PlaybackMode })}
                  >
                    <option value="never">Never</option>
                    {isMobile && <option value="cellular">When on cellular</option>}
                    <option value="remote">When remote</option>
                    {isMobile && (
                      <option value="remoteOrCellular">When remote or on cellular</option>
                    )}
                    <option value="always">Always</option>
                  </select>
                  {settings.playbackMode !== "never" && (
                    <select
                      className="sort-select"
                      value={settings.transcodeBitrate}
                      onChange={(e) =>
                        save({
                          transcodeBitrate: e.target.value as Settings["transcodeBitrate"],
                        })
                      }
                    >
                      <option value="kbps320">320 kbps</option>
                      <option value="kbps256">256 kbps</option>
                      <option value="kbps192">192 kbps</option>
                      <option value="kbps128">128 kbps</option>
                    </select>
                  )}
                </div>
              </div>
              <HelperText>{MODE_PROSE[settings.playbackMode]}</HelperText>

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
              <HelperText>
                How many upcoming tracks to download in advance, so the next song is always ready.
              </HelperText>

              {!isMobile && (
                <>
                  <label className="settings-row">
                    <span>Enable visualiser</span>
                    <input
                      type="checkbox"
                      checked={!settings.disableSpectrum}
                      onChange={(e) => {
                        if (e.target.checked) {
                          setShowSpectrumConfirm(true);
                        } else {
                          save({ disableSpectrum: true });
                        }
                      }}
                    />
                  </label>
                  <HelperText>
                    Renders a graphic visualiser behind the album art in focus mode. calculated once
                    per track at play time.
                  </HelperText>
                </>
              )}
            </>
          )}

          {activeTab === "library" && (
            <>
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
              <HelperText>
                Background quick-sync cadence. Set to Off to only sync when you press one of the
                buttons below.
              </HelperText>

              {!isMobile && (
                <>
                  <label className="settings-row">
                    <span>Library Padding ({settings.libraryPadding + 8})</span>
                    <input
                      type="range"
                      min={-8}
                      max={8}
                      value={settings.libraryPadding}
                      onChange={(e) => save({ libraryPadding: Number(e.target.value) })}
                    />
                  </label>
                  <HelperText>
                    Adjusts the spacing around library items so the layout feels denser or roomier.
                  </HelperText>
                </>
              )}

              <label className="settings-row">
                <span>Track popularity</span>
                <select
                  className="sort-select"
                  value={settings.popularityDisplay}
                  onChange={(e) =>
                    save({
                      popularityDisplay: e.target.value as Settings["popularityDisplay"],
                    })
                  }
                >
                  <option value="off">Off</option>
                  <option value="hot">Hot tracks</option>
                  <option value="chart">Popularity chart</option>
                </select>
              </label>
              <HelperText>
                This data comes directly from Plex, based on how many people have starred each
                track.
                <br />
                <br />
                <strong>Hot Tracks</strong> shows the most popular tracks per album. Bigger album =
                more tracks
                <br />
                <strong>Popularity chart</strong> shows the relative popularity of all tracks,
                versus the most popular one.
              </HelperText>

              <label className="settings-row">
                <span>Show artist flags</span>
                <input
                  type="checkbox"
                  checked={settings.showArtistFlags}
                  onChange={(e) => save({ showArtistFlags: e.target.checked })}
                />
              </label>

              <div className="settings-section-header">SYNC</div>

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
              <HelperText>
                <strong>Quick Sync</strong> catches anything new since your last sync, very quick.
                Does nothing if nothing changed
                <br />
                <strong>Genre Sync</strong> re-fetches just the genre tags, useful after
                editing/updating tags in Plex.
                <br />
                <strong>Full Sync</strong> re-reads your entire library from scratch, only needed if
                something's missing or broken.
              </HelperText>

              {stats && stats.trackCount > 0 && (
                <div className="settings-stats">
                  {stats.artistCount} artists, {stats.albumCount} albums, {stats.trackCount} tracks,{" "}
                  {stats.genreCount} genres
                </div>
              )}

              <div className="settings-section-header">GENRES</div>

              <label className="settings-row">
                <span>Flat genres</span>
                <input
                  type="checkbox"
                  checked={settings.flatGenres}
                  onChange={(e) => save({ flatGenres: e.target.checked })}
                />
              </label>
              <HelperText>
                Show every genre as a flat alphabetical list instead of a hierarchical tree.
              </HelperText>

              <div className="settings-row">
                <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                  <button
                    className={`settings-btn${settings.genreSource === "open" ? " active" : ""}`}
                    onClick={() => switchGenreSource("open")}
                  >
                    Open Source
                  </button>
                  <button
                    className={`settings-btn${settings.genreSource === "custom" ? " active" : ""}`}
                    disabled={!hasCustomGenres}
                    title={!hasCustomGenres ? "Import custom genres first" : undefined}
                    onClick={() => switchGenreSource("custom")}
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
            </>
          )}

          {activeTab === "storage" && (
            <>
              <AudioCacheRow
                limitBytes={settings.audioCacheLimitBytes}
                onLimitChange={(b) => save({ audioCacheLimitBytes: b })}
              />
              <ImageCacheRow
                limitBytes={settings.imageCacheLimitBytes}
                onLimitChange={(b) => save({ imageCacheLimitBytes: b })}
              />

              <div className="settings-section-header">DOWNLOADS</div>

              <button className="settings-btn" onClick={onOpenDownloads}>
                Manage downloads
              </button>

              <label className="settings-row">
                <span>Offline mode</span>
                <input
                  type="checkbox"
                  checked={settings.offlineMode}
                  onChange={(e) => save({ offlineMode: e.target.checked })}
                />
              </label>
              <HelperText>
                Hides anything you don't have downloaded and skips your server connection entirely.
                Useful when you know you'll be offline (flight, train etc) or want to ration data.
              </HelperText>
            </>
          )}

          {activeTab === "network" && (
            <>
              <label className="settings-row">
                <span>Refuse HTTP connections</span>
                <input
                  type="checkbox"
                  checked={settings.refuseHttp}
                  onChange={(e) => save({ refuseHttp: e.target.checked })}
                />
              </label>
              <HelperText>
                Forces HTTPS for every connection to your Plex server. It's better to handle your
                plex security preferences directly from your server settings. But if you just want
                to stop this client from ever trying a non HTTPS connection, this will force it.
              </HelperText>
            </>
          )}

          {activeTab === "about" && (
            <>
              <button className="settings-btn" onClick={() => setShowAcknowledgements(true)}>
                Acknowledgements &amp; licenses
              </button>

              <div className="settings-section-header">ACCOUNT</div>

              <button className="settings-btn settings-signout" onClick={handleSignOut}>
                Sign Out
              </button>
            </>
          )}
        </div>
      </div>
      {showAcknowledgements && (
        <AcknowledgementsPanel onDismiss={() => setShowAcknowledgements(false)} />
      )}
      {showSpectrumConfirm && (
        <ConfirmDialog
          message="The visualiser uses a small amount of cpu to analyse your music. Are you sure?"
          onConfirm={() => {
            save({ disableSpectrum: false });
            setShowSpectrumConfirm(false);
          }}
          onCancel={() => setShowSpectrumConfirm(false)}
        />
      )}
    </div>
  );
}

interface SyncProgressBannerProps {
  progress: SyncProgress;
}

function SyncProgressBanner({ progress }: SyncProgressBannerProps) {
  const done = progress.phase === "done";
  const pct =
    progress.total > 0 ? Math.min(100, (progress.current / progress.total) * 100) : done ? 100 : 0;
  return (
    <div className={`sync-banner${done ? " done" : ""}`}>
      <div className="sync-banner-fill" style={{ width: `${pct}%` }} />
      <div className="sync-banner-caption">
        {done ? "Sync complete" : progress.detail || "Syncing…"}
      </div>
    </div>
  );
}

interface ConfirmDialogProps {
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}

function ConfirmDialog({ message, onConfirm, onCancel }: ConfirmDialogProps) {
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onCancel]);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onCancel();
  };

  return (
    <div className="settings-backdrop" onClick={handleBackdropClick}>
      <div className="confirm-dialog glass">
        <div className="confirm-dialog-message">{message}</div>
        <div className="confirm-dialog-actions">
          <button className="settings-btn" onClick={onCancel}>
            No
          </button>
          <button className="settings-btn active" onClick={onConfirm}>
            Yes
          </button>
        </div>
      </div>
    </div>
  );
}
