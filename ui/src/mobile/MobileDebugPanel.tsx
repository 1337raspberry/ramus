import { useCallback, useEffect, useState } from "react";
import { getDebugInfo, type DebugInfo } from "../lib/commands";
import { usePlaybackStore } from "../stores/playbackStore";
import { useConnectionStatus } from "../lib/useConnectionStatus";

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const SOURCE_LABELS: Record<string, string> = {
  downloaded: "Downloaded file",
  cached: "Prefetch cache",
  transcode: "HLS transcode",
  streaming: "Direct stream",
  none: "Nothing playing",
};

const MODE_LABELS: Record<string, string> = {
  directPlay: "Direct Play",
  transcodeLosslessRemote: "Transcode if Remote",
  transcodeLossless: "Always Transcode",
};

// Strip credentials from a Plex URL before display. Direct-play URLs
// carry `?X-Plex-Token=…` and transcode URLs nest the token inside an
// `X-Plex-Headers=…` value, so screenshotting the panel as-is would
// leak the user's server token.
function redactUrl(url: string): string {
  return url.replace(/(X-Plex-Token=|X-Plex-Headers=)[^&#]*/gi, "$1<redacted>");
}

export default function MobileDebugPanel({ onDismiss }: { onDismiss: () => void }) {
  const [info, setInfo] = useState<DebugInfo | null>(null);
  const track = usePlaybackStore((s) => s.currentTrack);
  const status = usePlaybackStore((s) => s.status);
  const position = usePlaybackStore((s) => s.position);
  const connection = useConnectionStatus();

  const refresh = useCallback(() => {
    getDebugInfo()
      .then(setInfo)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 2000);
    return () => clearInterval(id);
  }, [refresh]);

  return (
    <div className="debug-panel-backdrop" onClick={onDismiss}>
      <div className="debug-panel" onClick={(e) => e.stopPropagation()}>
        <div className="debug-panel-header">
          <span>Debug</span>
          <button className="debug-panel-close" onClick={onDismiss}>
            &times;
          </button>
        </div>

        <div className="debug-panel-body">
          <Section title="Now Playing">
            <Row label="Track" value={track ? `${track.title}` : "—"} />
            <Row label="Artist" value={track?.artistName ?? "—"} />
            <Row
              label="Status"
              value={status}
              tag={status === "playing" ? "green" : status === "paused" ? "yellow" : "dim"}
            />
            <Row label="Position" value={position != null ? `${position.toFixed(1)}s` : "—"} />
          </Section>

          <Section title="Playback Engine">
            <Row
              label="Source"
              value={info ? (SOURCE_LABELS[info.source] ?? info.source) : "…"}
              tag={
                info?.source === "downloaded"
                  ? "green"
                  : info?.source === "cached"
                    ? "blue"
                    : info?.source === "transcode"
                      ? "yellow"
                      : info?.source === "streaming"
                        ? "orange"
                        : "dim"
              }
            />
            <Row label="Codec" value={info?.codec?.toUpperCase() ?? "—"} />
            <Row label="Bitrate" value={info?.bitrate ? `${info.bitrate} kbps` : "—"} />
            <Row label="File size" value={formatBytes(info?.fileSizeBytes)} />
            <Row label="Loading" value={info?.isLoading ? "yes" : "no"} />
            <Row
              label="Mode"
              value={info ? (MODE_LABELS[info.playbackMode] ?? info.playbackMode) : "…"}
            />
          </Section>

          <Section title="Connection">
            <Row label="Server" value={info?.serverUrl ?? "—"} mono />
            <Row
              label="Remote"
              value={info?.isRemote ? "yes" : "no"}
              tag={info?.isRemote ? "orange" : "green"}
            />
            <Row
              label="Online"
              value={connection.online ? "yes" : "no"}
              tag={connection.online ? "green" : "red"}
            />
            <Row
              label="Eff. offline"
              value={connection.effectiveOffline ? "yes" : "no"}
              tag={connection.effectiveOffline ? "red" : "dim"}
            />
          </Section>

          <Section title="Queue & Cache">
            <Row label="Queue" value={info ? `${info.queueIndex + 1} / ${info.queueLen}` : "…"} />
            <Row
              label="Lookahead"
              value={
                info
                  ? `${info.cachedInLookahead} / ${info.totalInLookahead} cached (depth ${info.lookaheadDepth})`
                  : "…"
              }
            />
          </Section>

          {info?.resolvedUrl && (
            <Section title="Resolved URL">
              <div className="debug-url">{redactUrl(info.resolvedUrl)}</div>
            </Section>
          )}
        </div>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="debug-section">
      <div className="debug-section-title">{title}</div>
      {children}
    </div>
  );
}

function Row({
  label,
  value,
  tag,
  mono,
}: {
  label: string;
  value: string;
  tag?: string;
  mono?: boolean;
}) {
  return (
    <div className="debug-row">
      <span className="debug-label">{label}</span>
      <span className={`debug-value${tag ? ` debug-tag-${tag}` : ""}${mono ? " debug-mono" : ""}`}>
        {value}
      </span>
    </div>
  );
}
