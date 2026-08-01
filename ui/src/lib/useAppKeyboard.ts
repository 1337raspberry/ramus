import { useCallback, useEffect } from "react";
import { togglePlayPause, nextTrack, previousTrack } from "./commands";
import { useGenreInfoStore } from "../stores/genreInfoStore";
import { usePlaybackStore } from "../stores/playbackStore";

interface UseAppKeyboardParams {
  setShowSearch: React.Dispatch<React.SetStateAction<boolean>>;
  setSearchInitial: React.Dispatch<React.SetStateAction<string | undefined>>;
  setShowEQ: React.Dispatch<React.SetStateAction<boolean>>;
  setShowSettings: React.Dispatch<React.SetStateAction<boolean>>;
  toggleFocusMode: () => void;
}

/**
 * Global keyboard shortcuts for the app shell.
 */
export function useAppKeyboard({
  setShowSearch,
  setSearchInitial,
  setShowEQ,
  setShowSettings,
  toggleFocusMode,
}: UseAppKeyboardParams): void {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      // While the genre-info modal/sheet is open, ALL shell shortcuts yield:
      // Escape belongs to the modal's own listener (this one registers
      // earlier and would exit focus mode underneath it), and the rest
      // (search operators, Cmd+F/E/,/N, Space) would open sibling overlays
      // BEHIND the modal's z-1100 backdrop or mutate playback invisibly.
      if (useGenreInfoStore.getState().target) return;

      // Esc exits focus mode before any other Esc-based dismissal.
      if (e.key === "Escape" && usePlaybackStore.getState().isFocusMode) {
        e.preventDefault();
        toggleFocusMode();
        return;
      }

      // Cmd/Ctrl+Shift+N toggles focus Now Playing. `e.key` is uppercase
      // while Shift is held.
      if (mod && e.shiftKey && e.key === "N") {
        e.preventDefault();
        toggleFocusMode();
        return;
      }

      if (mod && e.key === "f") {
        e.preventDefault();
        setSearchInitial(undefined);
        setShowSearch((s) => !s);
        return;
      }

      if (mod && e.key === "e") {
        e.preventDefault();
        setShowEQ((s) => !s);
        return;
      }

      if (mod && e.key === ",") {
        e.preventDefault();
        setShowSettings((s) => !s);
        return;
      }

      // Operator keys open search with that character pre-loaded.
      if (
        !mod &&
        !e.shiftKey &&
        "/!".includes(e.key) &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement)
      ) {
        e.preventDefault();
        setSearchInitial(e.key);
        setShowSearch(true);
        return;
      }
      if (
        !mod &&
        e.shiftKey &&
        ["@", "%", "#"].includes(e.key) &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement)
      ) {
        e.preventDefault();
        setSearchInitial(e.key);
        setShowSearch(true);
        return;
      }

      if (
        e.key === " " &&
        !mod &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement)
      ) {
        e.preventDefault();
        togglePlayPause();
        return;
      }

      if (mod && e.key === "ArrowRight") {
        e.preventDefault();
        nextTrack();
        return;
      }

      if (mod && e.key === "ArrowLeft") {
        e.preventDefault();
        previousTrack();
        return;
      }
    },
    [setShowSearch, setShowEQ, setShowSettings, setSearchInitial, toggleFocusMode],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
