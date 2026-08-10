import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Album } from "../lib/types";
import {
  addAlbumToCollection,
  getAlbumCollections,
  getAllCollectionNames,
  removeAlbumFromCollection,
} from "../lib/commands";
import { useToastStore } from "../components/Toast";
import { pushBackHandler } from "../lib/backHandler";
import { IconCheck } from "../components/Icons";

interface Props {
  album: Album;
  /** Layer above the now-playing sheet (z-1100) when opened from it. */
  overSheet?: boolean;
  onDismiss: () => void;
}

/**
 * Action-sheet checklist for an album's collection memberships. Tapping an
 * unchecked collection adds the album to it; tapping a checked one removes
 * it — the sheet stays open so several can be toggled in one visit. Portals
 * to <body> like the other mobile action sheets — callers inside virtualizer
 * rows or the now-playing sheet can't host a fixed overlay themselves
 * (transformed ancestors become its containing block).
 */
export default function CollectionPickerSheet({ album, overSheet, onDismiss }: Props) {
  const [collections, setCollections] = useState<string[] | null>(null);
  const [memberOf, setMemberOf] = useState<Set<string>>(() => new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    Promise.all([getAllCollectionNames(), getAlbumCollections(album.ratingKey)])
      .then(([all, mine]) => {
        if (cancelled) return;
        setCollections(all);
        setMemberOf(new Set(mine.map((n) => n.toLowerCase())));
      })
      .catch(() => {
        if (!cancelled) setCollections([]);
      });
    return () => {
      cancelled = true;
    };
  }, [album.ratingKey]);

  // Hardware back closes the picker, not whatever sits underneath it.
  useEffect(
    () =>
      pushBackHandler(() => {
        onDismiss();
        return true;
      }),
    [onDismiss],
  );

  const toggle = (name: string) => {
    if (busy) return;
    setBusy(true);
    const key = name.toLowerCase();
    const isMember = memberOf.has(key);
    const action = isMember
      ? removeAlbumFromCollection(album.ratingKey, name)
      : addAlbumToCollection(album.ratingKey, name);
    action
      .then(() => {
        setMemberOf((prev) => {
          const next = new Set(prev);
          if (isMember) next.delete(key);
          else next.add(key);
          return next;
        });
        useToastStore.getState().show(isMember ? `Removed from ${name}` : `Added to ${name}`);
      })
      .catch(() => {
        useToastStore
          .getState()
          .show(isMember ? "Couldn't remove from collection" : "Couldn't add to collection");
      })
      .finally(() => setBusy(false));
  };

  return createPortal(
    <div
      className={`mobile-action-sheet-backdrop${overSheet ? " over-sheet" : ""}`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="mobile-action-sheet">
        <div className="mobile-action-sheet-group">
          <div className="mobile-action-sheet-header">
            Collections for “{album.title}” — tap to add or remove
          </div>
          <div className="mobile-collection-list">
            {collections === null ? (
              <div className="mobile-collection-empty">Loading…</div>
            ) : collections.length === 0 ? (
              <div className="mobile-collection-empty">No collections in your library yet</div>
            ) : (
              collections.map((name) => {
                const member = memberOf.has(name.toLowerCase());
                return (
                  <button key={name} disabled={busy} onClick={() => toggle(name)}>
                    <span className="mobile-collection-name">{name}</span>
                    {member && (
                      <span className="mobile-collection-check">
                        <IconCheck size={18} />
                      </span>
                    )}
                  </button>
                );
              })
            )}
          </div>
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Done
        </button>
      </div>
    </div>,
    document.body,
  );
}
