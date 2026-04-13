import { useCallback, useEffect } from "react";
import { togglePlayPause, nextTrack, previousTrack } from "./commands";
import { usePlaybackStore } from "../stores/playbackStore";

interface UseAppKeyboardParams {
  setShowSearch: React.Dispatch<React.SetStateAction<boolean>>;
  setSearchInitial: React.Dispatch<React.SetStateAction<string | undefined>>;
  setShowEQ: React.Dispatch<React.SetStateAction<boolean>>;
  setShowSettings: React.Dispatch<React.SetStateAction<boolean>>;
  setShowColorDebug: React.Dispatch<React.SetStateAction<boolean>>;
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
  setShowColorDebug,
  toggleFocusMode,
}: UseAppKeyboardParams): void {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;

      // Esc exits focus mode (before any other Esc-based dismissal)
      if (e.key === "Escape" && usePlaybackStore.getState().isFocusMode) {
        e.preventDefault();
        toggleFocusMode();
        return;
      }

      // Cmd/Ctrl+Shift+N toggles focus "Now Playing" mode
      // (with Shift held, e.key is always uppercase)
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

      if (mod && e.shiftKey && e.key === "D") {
        e.preventDefault();
        setShowColorDebug((s) => !s);
        return;
      }

      // Operator keys open search with that character pre-loaded
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
    [
      setShowSearch,
      setShowEQ,
      setShowSettings,
      setShowColorDebug,
      setSearchInitial,
      toggleFocusMode,
    ],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);
}
