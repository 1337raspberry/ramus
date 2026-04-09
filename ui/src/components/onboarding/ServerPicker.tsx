import { useCallback, useEffect, useState } from "react";
import { discoverServers, testServer, connectManualUrl } from "../../lib/commands";
import type { PlexServer } from "../../lib/types";

interface Props {
  onSelect: (server: PlexServer, serverUrl: string) => void;
}

interface ServerStatus {
  testing: boolean;
  connected: boolean;
  uri?: string;
  local?: boolean;
  isHttp?: boolean;
}

export default function ServerPicker({ onSelect }: Props) {
  const [servers, setServers] = useState<PlexServer[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Map<string, ServerStatus>>(new Map());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showManual, setShowManual] = useState(false);
  const [manualUrl, setManualUrl] = useState("");
  const [manualTesting, setManualTesting] = useState(false);

  useEffect(() => {
    discoverServers()
      .then((found) => {
        setServers(found);
        setLoading(false);
        found.forEach((server) => {
          setStatuses((prev) => {
            const next = new Map(prev);
            next.set(server.machineIdentifier, { testing: true, connected: false });
            return next;
          });
          testServer(server.machineIdentifier)
            .then((result) => {
              setStatuses((prev) => {
                const next = new Map(prev);
                next.set(server.machineIdentifier, {
                  testing: false,
                  connected: result.connected,
                  uri: result.uri,
                  local: result.local,
                  isHttp: result.isHttp,
                });
                return next;
              });
            })
            .catch(() => {
              setStatuses((prev) => {
                const next = new Map(prev);
                next.set(server.machineIdentifier, { testing: false, connected: false });
                return next;
              });
            });
        });
      })
      .catch((e) => {
        setError(String(e));
        setLoading(false);
      });
  }, []);

  const handleSelect = useCallback(
    (server: PlexServer) => {
      const status = statuses.get(server.machineIdentifier);
      if (!status?.connected || !status.uri) return;
      setSelectedId(server.machineIdentifier);
      onSelect(server, status.uri);
    },
    [statuses, onSelect],
  );

  const handleManualConnect = useCallback(async () => {
    if (!manualUrl.trim()) return;
    setManualTesting(true);
    try {
      const ok = await connectManualUrl(manualUrl);
      if (ok) {
        // Create a synthetic server entry for manual connection
        onSelect(
          {
            machineIdentifier: "manual",
            name: manualUrl,
            owned: true,
            connections: [{ uri: manualUrl, local: false, relay: false, protocol: "http" }],
          },
          manualUrl,
        );
      } else {
        setError("Could not connect to server");
      }
    } catch (e) {
      setError(String(e));
    }
    setManualTesting(false);
  }, [manualUrl, onSelect]);

  const connectionLabel = (status: ServerStatus) => {
    if (status.testing) return "Testing...";
    if (!status.connected) return "Unavailable";
    if (status.local) return "Local";
    if (status.isHttp) return "Remote (HTTP)";
    return "Remote";
  };

  return (
    <div className="onboarding-step">
      <h2>Select a Server</h2>
      <p className="onboarding-subtitle">Choose a Plex server with a music library.</p>

      {loading && <div className="onboarding-loading">Discovering servers...</div>}

      {!loading && servers.length === 0 && !error && (
        <div className="onboarding-empty">No servers found. Try entering a URL manually.</div>
      )}

      {error && <div className="onboarding-error">{error}</div>}

      <div className="server-list">
        {servers.map((server) => {
          const status = statuses.get(server.machineIdentifier);
          const isSelected = selectedId === server.machineIdentifier;
          return (
            <div
              key={server.machineIdentifier}
              className={`server-row${isSelected ? " selected" : ""}${status?.connected ? "" : " unavailable"}`}
              onClick={() => handleSelect(server)}
            >
              <span className="server-icon">{server.owned ? "\uD83D\uDDA5" : "\uD83D\uDD17"}</span>
              <div className="server-info">
                <div className="server-name">{server.name}</div>
                <div className="server-status">{status ? connectionLabel(status) : "..."}</div>
              </div>
              {status?.isHttp && (
                <span className="server-http-warn" title="Unencrypted connection">
                  {"\u26A0"}
                </span>
              )}
            </div>
          );
        })}
      </div>

      <button className="onboarding-text-btn" onClick={() => setShowManual(!showManual)}>
        {showManual ? "Hide manual entry" : "Enter URL manually"}
      </button>

      {showManual && (
        <div className="manual-url-row">
          <input
            className="manual-url-input"
            type="text"
            placeholder="https://your-server:32400"
            value={manualUrl}
            onChange={(e) => setManualUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleManualConnect();
            }}
          />
          <button
            className="onboarding-primary-btn"
            onClick={handleManualConnect}
            disabled={manualTesting}
          >
            {manualTesting ? "Testing..." : "Connect"}
          </button>
        </div>
      )}
    </div>
  );
}
