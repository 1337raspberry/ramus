import { usePlaybackQualityStore } from "../stores/playbackQualityStore";

interface Props {
  /** Opens the settings panel. When absent the notice is purely passive. */
  onOpenSettings?: () => void;
}

/**
 * Passive notice shown while the connection can't sustain what's playing,
 * and while a reduced bitrate is in force because of it.
 *
 * Persistent rather than a toast: this state lasts as long as the bad link
 * does, and a 3s auto-dismissing message is invisible to anyone who wasn't
 * looking at the screen at that moment.
 *
 * Renders nothing in the common case, so it's free to mount anywhere.
 */
export default function PlaybackQualityNotice({ onOpenSettings }: Props) {
  const starving = usePlaybackQualityStore((s) => s.starving);
  const degradedToKbps = usePlaybackQualityStore((s) => s.degradedToKbps);
  const adaptationBlocked = usePlaybackQualityStore((s) => s.adaptationBlocked);

  if (!starving && degradedToKbps == null) return null;

  let message: string;
  if (degradedToKbps != null) {
    message = starving
      ? `Still struggling at ${degradedToKbps} kbps`
      : `Reduced to ${degradedToKbps} kbps for this connection`;
  } else if (adaptationBlocked) {
    // The only case with nothing to report but the problem itself: the mode
    // forbids acting, so point at the setting that would allow it.
    message = "Connection too slow for lossless";
  } else {
    message = "Connection too slow — reducing quality";
  }

  return (
    <div className="quality-notice" role="status">
      <span className="quality-notice-text">{message}</span>
      {adaptationBlocked && onOpenSettings && (
        <button type="button" className="quality-notice-action" onClick={onOpenSettings}>
          Transcode settings
        </button>
      )}
    </div>
  );
}
