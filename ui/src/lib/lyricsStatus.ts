import type { LyricsStatus } from "./types";

/** Honest empty-state copy for a finished fetch that produced no lyrics. */
export function lyricsEmptyMessage(status: LyricsStatus | null): string {
  switch (status) {
    case "offline":
      return "Network unavailable";
    case "unreachable":
      return "Couldn't reach lyrics server";
    case "notFound":
      return "No lyrics found";
    default:
      return "No lyrics available";
  }
}
