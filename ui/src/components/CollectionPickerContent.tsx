import { useEffect, useState } from "react";
import type { Album } from "../lib/types";
import {
  addAlbumToCollection,
  getAlbumCollections,
  getAllCollectionNames,
  removeAlbumFromCollection,
} from "../lib/commands";
import { useToastStore } from "./Toast";
import { IconCheck } from "./Icons";

/**
 * Checklist body for an album's collection memberships, shared between the
 * mobile action sheet and the desktop modal (same core-plus-shells split as
 * GenreInfoContent). Tapping an unchecked collection adds the album to it;
 * tapping a checked one removes it — the surface stays open so several can
 * be toggled in one visit.
 */
export default function CollectionPickerContent({ album }: { album: Album }) {
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

  return (
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
  );
}
