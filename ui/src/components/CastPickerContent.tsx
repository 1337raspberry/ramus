import { useCallback, useEffect, useState } from "react";
import { listCastPlayers, startCast, stopCast } from "../lib/commands";
import type { CastPlayerView } from "../lib/types";
import { useCastStore } from "../stores/castStore";
import { IconCheck, IconSpinner } from "./Icons";

/**
 * The player list behind the cast button, shared by the desktop modal and
 * the mobile sheet. "This device" hands playback back; a player row moves
 * the queue there. Players that didn't answer their probe are dimmed.
 */
export default function CastPickerContent({ onDone }: { onDone: () => void }) {
  const activeId = useCastStore((s) => s.player?.id ?? null);
  const [players, setPlayers] = useState<CastPlayerView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The row being switched to: a player id, or "local" for this device.
  const [pending, setPending] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setPlayers(null);
    setError(null);
    listCastPlayers()
      .then((list) => {
        if (!cancelled) setPlayers(list);
      })
      .catch((e) => {
        if (cancelled) return;
        setPlayers([]);
        setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);

  const choose = (id: string | null) => {
    if (pending) return;
    if (id === activeId) {
      onDone();
      return;
    }
    setPending(id ?? "local");
    setError(null);
    (id ? startCast(id) : stopCast())
      .then(onDone)
      .catch((e) => setError(String(e)))
      .finally(() => setPending(null));
  };

  const mark = (rowId: string, checked: boolean) =>
    pending === rowId ? (
      <span className="mobile-collection-check">
        <IconSpinner size={18} />
      </span>
    ) : checked ? (
      <span className="mobile-collection-check">
        <IconCheck size={18} />
      </span>
    ) : null;

  return (
    <div className="mobile-collection-list cast-picker-list">
      <button disabled={pending !== null} onClick={() => choose(null)}>
        <span className="mobile-collection-name">This device</span>
        {mark("local", activeId === null)}
      </button>
      {players === null ? (
        <div className="mobile-collection-empty">Looking for players…</div>
      ) : players.length === 0 && !error ? (
        <div className="mobile-collection-empty">No other Plex players found</div>
      ) : (
        players.map((p) => (
          <button
            key={p.id}
            className={p.reachable ? undefined : "cast-picker-unreachable"}
            disabled={pending !== null || !p.reachable}
            onClick={() => choose(p.id)}
          >
            <span className="mobile-collection-name">
              {p.name}
              {p.product && <span className="cast-picker-product">{p.product}</span>}
            </span>
            {mark(p.id, activeId === p.id)}
          </button>
        ))
      )}
      {!error && players !== null && players.length > 0 && players.every((p) => !p.reachable) && (
        // Usually the network, or on iPhone a Local Network permission that
        // was still being asked for (or was refused) during the probe.
        <>
          <div className="mobile-collection-empty cast-picker-error">
            None of these players answered. Check they're on and on the same network. On iPhone,
            also check ramus has Local Network access in Settings.
          </div>
          <button onClick={retry}>Retry</button>
        </>
      )}
      {error && (
        <>
          <div className="mobile-collection-empty cast-picker-error">{error}</div>
          <button onClick={retry}>Retry</button>
        </>
      )}
    </div>
  );
}
