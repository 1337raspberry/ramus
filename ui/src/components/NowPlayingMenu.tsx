import { useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import type { Album } from "../lib/types";
import CollectionPickerModal from "./CollectionPickerModal";
import PlaylistPickerModal from "./PlaylistPickerModal";
import { IconMoreDots } from "./Icons";

/**
 * The desktop player's … overflow menu, shared by the compact panel and
 * focus mode. Carries the actions with no inline affordance — collections,
 * playlists, saving or clearing the queue (artist/album navigation and the
 * favourite stars are already clickable in both hosts). The picker modals
 * portal to <body> and out-z the focus overlay, so one component serves
 * both contexts.
 */
/** Rough height of the fully-populated menu, plus a little breathing room —
 * the threshold for flipping it above the button. */
const MENU_CLEARANCE = 170;

export default function NowPlayingMenu() {
  const track = usePlaybackStore((s) => s.currentTrack);
  const nowPlayingAlbum = usePlaybackStore((s) => s.nowPlayingAlbum);
  const hasQueue = usePlaybackStore((s) => s.queue.length > 0);
  const clearQueue = usePlaybackStore((s) => s.clearQueue);
  const [menuOpen, setMenuOpen] = useState(false);
  const [openUp, setOpenUp] = useState(false);
  // Both pickers resolve their target when the menu item is tapped, never at
  // confirm time. The live selectors above re-run on every track change, so a
  // natural advance while the picker sits open would silently retarget it —
  // the user taps a playlist and the track that happens to be playing now
  // gets added instead of the one they opened the menu for.
  const [collectionAlbum, setCollectionAlbum] = useState<Album | null>(null);
  const [playlistTarget, setPlaylistTarget] = useState<{
    heading: string;
    ids: string[];
    createOnly?: boolean;
  } | null>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".np-menu-wrap")) setMenuOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  if (!track) return null;

  return (
    <div className="np-menu-wrap">
      <button
        ref={btnRef}
        className="np-eq-btn"
        onClick={() => {
          // The compact panel sits this button near the bottom of the
          // window, where a downward menu would be clipped; focus mode puts
          // it near the top. Measure at open time rather than branching on
          // the host.
          const rect = btnRef.current?.getBoundingClientRect();
          if (rect) setOpenUp(window.innerHeight - rect.bottom < MENU_CLEARANCE);
          setMenuOpen((v) => !v);
        }}
        title="More actions"
        aria-haspopup="menu"
      >
        <IconMoreDots />
      </button>
      {menuOpen && (
        <div className={`adv-dropdown np-menu-dropdown${openUp ? " up" : ""}`}>
          {nowPlayingAlbum && (
            <button
              onClick={() => {
                setMenuOpen(false);
                setCollectionAlbum(nowPlayingAlbum);
              }}
            >
              Add Album to Collection…
            </button>
          )}
          <button
            onClick={() => {
              setMenuOpen(false);
              setPlaylistTarget({ heading: track.title, ids: [track.ratingKey] });
            }}
          >
            Add Track to Playlist…
          </button>
          {hasQueue && (
            <button
              onClick={() => {
                setMenuOpen(false);
                setPlaylistTarget({
                  heading: "Queue",
                  ids: usePlaybackStore.getState().queue.map((t) => t.ratingKey),
                  createOnly: true,
                });
              }}
            >
              Save Queue as Playlist…
            </button>
          )}
          <button
            className="destructive"
            onClick={() => {
              setMenuOpen(false);
              clearQueue();
            }}
          >
            Clear Queue
          </button>
        </div>
      )}
      {collectionAlbum && (
        <CollectionPickerModal album={collectionAlbum} onDismiss={() => setCollectionAlbum(null)} />
      )}
      {playlistTarget && (
        <PlaylistPickerModal
          heading={playlistTarget.heading}
          createOnly={playlistTarget.createOnly}
          getTrackIds={() => Promise.resolve(playlistTarget.ids)}
          onDismiss={() => setPlaylistTarget(null)}
        />
      )}
    </div>
  );
}
