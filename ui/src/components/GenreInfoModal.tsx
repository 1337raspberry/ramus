import { useEffect } from "react";
import { createPortal } from "react-dom";

import GenreInfoContent from "./GenreInfoContent";
import { IconChevronLeft } from "./Icons";
import { useGenreInfoStack } from "../lib/useGenreInfoStack";
import { useGenreInfoStore } from "../stores/genreInfoStore";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";

/**
 * Desktop counterpart of the mobile GenreInfoSheet: a centered, scrollable
 * settings-style modal for the richer genre metadata. Opened by right-clicking
 * any genre surface (via `genreInfoStore`); mounted once in the desktop app
 * root. Same drill/click semantics as mobile via the shared stack + content.
 */
export default function GenreInfoModal() {
  const target = useGenreInfoStore((s) => s.target);
  const close = useGenreInfoStore((s) => s.close);
  const selectGenreByName = useLibraryStore((s) => s.selectGenreByName);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);

  const { current, meta, loading, canGoBack, drillInto, goBack } = useGenreInfoStack(target);

  // Escape closes the whole modal (the header back button covers drill pops).
  // `useAppKeyboard` yields to us while the modal is open.
  useEffect(() => {
    if (!target) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [target, close]);

  if (!target || !current) return null;

  const titleInLibrary = meta?.inLibrary ?? false;

  // Navigating away: drop focus mode too, so the destination grid is
  // actually visible (the desktop analogue of mobile's `onNavigate`).
  const beforeNavigate = () => {
    close();
    if (usePlaybackStore.getState().isFocusMode) {
      usePlaybackStore.setState({ isFocusMode: false });
    }
  };

  const navigateToGenre = (genre: string) => {
    beforeNavigate();
    void selectGenreByName(genre);
  };

  const navigateToArtist = (artist: string) => {
    beforeNavigate();
    void loadAlbumsForArtistName(artist);
  };

  return createPortal(
    <div
      className="settings-backdrop genre-info-modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="settings-panel glass genre-info-modal" role="dialog" aria-modal="true">
        <div className="genre-info-modal-header">
          {canGoBack && (
            <button className="genre-info-back" onClick={goBack} aria-label="Back">
              <IconChevronLeft size={22} />
            </button>
          )}
          {titleInLibrary ? (
            <button
              className="genre-info-title linked"
              onClick={() => navigateToGenre(current)}
              title="Show albums for this genre"
            >
              {meta?.canonicalName ?? current}
            </button>
          ) : (
            <span className="genre-info-title">{meta?.canonicalName ?? current}</span>
          )}
          <button className="settings-close" onClick={close} aria-label="Close">
            x
          </button>
        </div>

        <div className="genre-info-modal-body">
          <GenreInfoContent
            meta={meta}
            loading={loading}
            onDrillGenre={drillInto}
            onNavigateArtist={navigateToArtist}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}
