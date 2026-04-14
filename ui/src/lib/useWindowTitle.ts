import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { usePlaybackStore } from "../stores/playbackStore";

const DEFAULT_TITLE = "ramus";
const appWindow = getCurrentWindow();

/**
 * Keeps the OS window title in sync with the current track. Shows up in the
 * Windows/Linux taskbar and in macOS Mission Control / Cmd-Tab even though our
 * titlebar is hidden there.
 *
 * Selects artistName/title as primitives rather than the whole `currentTrack`
 * object — playback-state events replace the object reference on every pause,
 * resume, and stop, which would otherwise fire a redundant setTitle each time.
 */
export function useWindowTitle(): void {
  const artistName = usePlaybackStore((s) => s.currentTrack?.artistName ?? null);
  const title = usePlaybackStore((s) => s.currentTrack?.title ?? null);

  useEffect(() => {
    const display = artistName && title ? `${artistName} \u2014 ${title}` : DEFAULT_TITLE;
    appWindow.setTitle(display).catch(() => {});
  }, [artistName, title]);
}
