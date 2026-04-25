import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import {
  ART_SIZE,
  setAlbumPalette,
  togglePlayPause,
  nextTrack,
  previousTrack,
  getQueue,
  getArtUrl,
} from "../lib/commands";
import { formatDuration } from "../lib/format";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { applyAccent } from "../lib/accent";
import { useArtUrl } from "../lib/useArtUrl";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "../components/WaveformSeekBar";
import FlowLayout from "../components/FlowLayout";
import UltraBlurBackground from "../components/UltraBlurBackground";
import MarqueeText from "../components/MarqueeText";
import LyricsOverlay from "../components/LyricsOverlay";

import {
  IconPlay,
  IconPause,
  IconPrevious,
  IconNext,
  IconStarFilled,
  IconStarEmpty,
  IconMusicNote,
  IconChevronDown,
  IconClose,
  IconEqualizer,
} from "../components/Icons";
import EqualizerPanel from "../components/EqualizerPanel";

function IconSkipBack({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 6l-9 6 9 6V6z" />
      <path d="M22 6l-9 6 9 6V6z" />
    </svg>
  );
}

function IconSkipForward({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M2 6l9 6-9 6V6z" />
      <path d="M12 6l9 6-9 6V6z" />
    </svg>
  );
}

function IconQuote() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
      <path d="M7 17h2l2-4V7H5v6h2l-1 4zm8 0h2l2-4V7h-6v6h2l-1 4z" />
    </svg>
  );
}

function UpNextThumb({ thumb }: { thumb: string | null }) {
  const [src, setSrc] = useState<string | null>(null);
  const [err, setErr] = useState(false);

  useEffect(() => {
    if (!thumb) return;
    let cancelled = false;
    getArtUrl(thumb, ART_SIZE.SMALL)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [thumb]);

  if (src && !err) {
    return <img className="mobile-upnext-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className="mobile-upnext-thumb mobile-upnext-thumb-ph">
      <IconMusicNote size={14} />
    </div>
  );
}

interface Props {
  expanded: boolean;
  onExpand: () => void;
  onCollapse: () => void;
}

/**
 * Mobile now-playing: bottom mini-player when collapsed, full-screen sheet
 * when expanded. Tap the mini-player to expand; chevron-down collapses.
 *
 * Both states share WaveformSeekBar so the cached offscreen shape is
 * rendered once and the progress overlay is cheap. Album-art palette
 * extraction runs once per track on the expanded hero image.
 */
export default function MobileNowPlaying({ expanded, onExpand, onCollapse }: Props) {
  const status = usePlaybackStore((s) => s.status);
  const currentGenres = usePlaybackStore((s) => s.currentGenres);
  const sheetBlurColors = usePlaybackStore((s) => s.ultraBlurColors);
  const queue = usePlaybackStore((s) => s.queue);
  const queueIndex = usePlaybackStore((s) => s.queueIndex);
  const jumpToIndex = usePlaybackStore((s) => s.jumpToIndex);
  const removeQueueItem = usePlaybackStore((s) => s.removeQueueItem);

  const {
    track,
    nowPlayingAlbum,
    hasTrackArtist,
    year,
    codecParts,
    albumFav,
    trackFav,
    handleAlbumFavToggle,
    handleTrackFavToggle,
    handleArtistClick,
    handleAlbumClick,
    handleGenreClick,
  } = useNowPlayingActions({ onNavigate: onCollapse });

  const thumb = track?.thumb ?? nowPlayingAlbum?.thumb ?? null;
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.LARGE);
  const lastAccentThumb = useRef<string | null>(null);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      const img = e.currentTarget;
      if (lastAccentThumb.current === thumb) return;
      lastAccentThumb.current = thumb;
      const capturedThumb = thumb;
      const existing = usePlaybackStore.getState().vibrantPalette;
      if (existing) {
        const [r, g, b] = accentFromPalette(existing);
        applyAccent(r, g, b);
        return;
      }
      extractPalette(img).then((palette) => {
        if (!palette || lastAccentThumb.current !== capturedThumb) return;
        const [r, g, b] = accentFromPalette(palette);
        applyAccent(r, g, b);
        const blurColors = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blurColors });
        if (track?.albumKey) {
          setAlbumPalette(track.albumKey, palette).catch(() => {});
        }
      });
    },
    [thumb, track?.albumKey],
  );

  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const showLyrics = usePlaybackStore((s) => s.showLyrics);
  const [showEQ, setShowEQ] = useState(false);

  // --- Swipe gestures ---
  // Mini-player: swipe up to expand. Sheet header: swipe down to collapse.
  // Both use the imperative non-passive touchmove pattern (same as the
  // album-art drag below) instead of React pointer events. Android
  // Chromium WebView cancels pointer events the moment it decides a
  // vertical drag is a scroll gesture, so a React-pointer approach
  // silently fails on Android even though it works on iOS WebKit.
  const SWIPE_THRESHOLD = 50;
  const [dragDeltaY, setDragDeltaY] = useState(0);
  const miniRef = useRef<HTMLDivElement>(null);
  const miniDragYRef = useRef(0);

  useEffect(() => {
    const el = miniRef.current;
    if (!el) return;
    let startY: number | null = null;
    let claimY = 0;
    let claimed = false;
    let skip = false;

    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) {
        skip = true;
        return;
      }
      const target = e.target as HTMLElement | null;
      // Skip drags that start on an interactive child (controls, waveform
      // scrubber, art button) so taps and scrubs aren't intercepted.
      skip = !!target?.closest(
        'button, [role="button"], input, .mobile-miniplayer-wave, .mobile-miniplayer-controls',
      );
      startY = e.touches[0].clientY;
      claimed = false;
    };

    const onMove = (e: TouchEvent) => {
      if (startY == null || skip) return;
      const y = e.touches[0].clientY;
      const dy = y - startY;
      if (!claimed) {
        if (dy < -3) {
          claimed = true;
          claimY = y;
        } else {
          return;
        }
      }
      e.preventDefault();
      const dragY = Math.min(0, y - claimY);
      if (miniDragYRef.current !== dragY) {
        miniDragYRef.current = dragY;
        setDragDeltaY(dragY);
      }
    };

    const onEnd = () => {
      if (claimed) {
        const finalDragY = miniDragYRef.current;
        miniDragYRef.current = 0;
        setDragDeltaY(0);
        if (finalDragY < -SWIPE_THRESHOLD) onExpand();
      }
      startY = null;
      claimed = false;
      skip = false;
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd, { passive: true });
    el.addEventListener("touchcancel", onEnd, { passive: true });
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, [onExpand]);

  const [sheetDragY, setSheetDragY] = useState(0);
  const [dismissing, setDismissing] = useState(false);
  const sheetRef = useRef<HTMLDivElement>(null);
  const sheetBodyRef = useRef<HTMLDivElement>(null);
  const sheetHeaderRef = useRef<HTMLElement>(null);
  const sheetDragYRef = useRef(0);
  const [mainMinHeight, setMainMinHeight] = useState<number | undefined>(undefined);

  useEffect(() => {
    if (!expanded) return;
    const el = sheetBodyRef.current;
    if (el) setMainMinHeight(el.clientHeight);
  }, [expanded, track?.ratingKey]);

  useEffect(() => {
    const el = sheetHeaderRef.current;
    if (!el) return;
    let startY: number | null = null;
    let claimY = 0;
    let claimed = false;
    let skip = false;

    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) {
        skip = true;
        return;
      }
      const target = e.target as HTMLElement | null;
      // Skip drags starting on the favourite button so the star tap
      // doesn't get swallowed.
      skip = !!target?.closest('button, [role="button"]');
      startY = e.touches[0].clientY;
      claimed = false;
    };

    const onMove = (e: TouchEvent) => {
      if (startY == null || skip) return;
      const y = e.touches[0].clientY;
      const dy = y - startY;
      if (!claimed) {
        if (dy > 3) {
          claimed = true;
          claimY = y;
        } else {
          return;
        }
      }
      e.preventDefault();
      const dragY = Math.max(0, y - claimY);
      if (sheetDragYRef.current !== dragY) {
        sheetDragYRef.current = dragY;
        setSheetDragY(dragY);
      }
    };

    const onEnd = () => {
      if (claimed) {
        const finalDragY = sheetDragYRef.current;
        sheetDragYRef.current = 0;
        if (finalDragY > SWIPE_THRESHOLD) {
          setSheetDragY(0);
          setDismissing(true);
        } else {
          setSheetDragY(0);
        }
      }
      startY = null;
      claimed = false;
      skip = false;
    };

    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: false });
    el.addEventListener("touchend", onEnd, { passive: true });
    el.addEventListener("touchcancel", onEnd, { passive: true });
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, []);

  // Drag-to-dismiss on the album art. We let native touch-scrolling handle
  // upward drags and downward drags while the body is scrolled (so momentum
  // and inertial flick work as the user expects). We only intercept the
  // gesture (preventDefault) once we observe a downward drag while the body
  // is at scrollTop=0 — at which point the gesture transitions seamlessly
  // into a sheet-dismiss preview, even if it started as a body scroll-back.
  const artRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const art = artRef.current;
    if (!art) return;

    let startY: number | null = null;
    let claimY = 0;
    let claimed = false;

    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) return;
      startY = e.touches[0].clientY;
      claimed = false;
    };

    const onMove = (e: TouchEvent) => {
      if (startY == null) return;
      const y = e.touches[0].clientY;
      if (!claimed) {
        // Defer to the lyrics overlay when it's mounted — it owns its own
        // internal scroll + tap-to-seek and we don't want preventDefault
        // on the outer art container eating those gestures.
        if (usePlaybackStore.getState().showLyrics) return;
        const dy = y - startY;
        const atTop = (sheetBodyRef.current?.scrollTop ?? 0) <= 0;
        if (dy > 3 && atTop) {
          claimed = true;
          claimY = y;
        } else {
          return;
        }
      }
      // Only safe because the listener is registered as { passive: false }.
      e.preventDefault();
      const dragY = Math.max(0, y - claimY);
      if (sheetDragYRef.current !== dragY) {
        sheetDragYRef.current = dragY;
        setSheetDragY(dragY);
      }
    };

    const onEnd = () => {
      if (claimed) {
        const finalDragY = sheetDragYRef.current;
        sheetDragYRef.current = 0;
        if (finalDragY > SWIPE_THRESHOLD) {
          setSheetDragY(0);
          setDismissing(true);
        } else {
          setSheetDragY(0);
        }
      }
      startY = null;
      claimed = false;
    };

    art.addEventListener("touchstart", onStart, { passive: true });
    art.addEventListener("touchmove", onMove, { passive: false });
    art.addEventListener("touchend", onEnd, { passive: true });
    art.addEventListener("touchcancel", onEnd, { passive: true });
    return () => {
      art.removeEventListener("touchstart", onStart);
      art.removeEventListener("touchmove", onMove);
      art.removeEventListener("touchend", onEnd);
      art.removeEventListener("touchcancel", onEnd);
    };
  }, []);

  const onSheetTransitionEnd = useCallback(
    (e: React.TransitionEvent) => {
      if (e.propertyName === "transform" && dismissing) {
        setDismissing(false);
        onCollapse();
      }
    },
    [dismissing, onCollapse],
  );

  useEffect(() => {
    if (expanded) {
      getQueue()
        .then((q) => usePlaybackStore.setState({ queue: q }))
        .catch(() => {});
    }
  }, [expanded]);

  // Close the sheet on Escape (iOS keyboard / external keyboard).
  useEffect(() => {
    if (!expanded) return;
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCollapse();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [expanded, onCollapse]);

  if (!track) return null;

  const isPlaying = status === "playing";
  const albumYear = year ? ` (${year})` : "";

  return (
    <>
      {/* Mini-player: always mounted to keep the waveform offscreen shape
          warm, hidden when expanded so taps hit the sheet. */}
      <div
        ref={miniRef}
        className="mobile-miniplayer"
        style={{
          ...(dragDeltaY !== 0 ? { transform: `translateY(${dragDeltaY}px)` } : {}),
          paddingBottom: 40,
        }}
      >
        {sheetBlurColors && (
          <div className="mobile-miniplayer-bg">
            <UltraBlurBackground colors={sheetBlurColors} />
            <div className="mobile-miniplayer-darken" style={{ background: "rgba(0,0,0,0.3)" }} />
          </div>
        )}
        <div className="mobile-miniplayer-hint" style={{ paddingTop: 10 }}>
          <div className="mobile-miniplayer-hint-pill" style={{ width: 50 }} />
        </div>
        <div className="mobile-miniplayer-bar" style={{ padding: "4px 14px 4px", gap: 0 }}>
          <div className="mobile-miniplayer-info">
            <MarqueeText className="mobile-miniplayer-title">{track.title}</MarqueeText>
            <div className="mobile-miniplayer-artist">{track.artistName}</div>
          </div>
          <div
            className="mobile-miniplayer-controls"
            onClick={(e) => e.stopPropagation()}
            onPointerDown={(e) => e.stopPropagation()}
          >
            <button
              className="mobile-miniplayer-btn"
              onClick={() => previousTrack().catch(() => {})}
              aria-label="Previous"
            >
              <IconSkipBack size={22} />
            </button>
            <button
              className="mobile-miniplayer-btn"
              onClick={() => togglePlayPause().catch(() => {})}
              aria-label={isPlaying ? "Pause" : "Play"}
            >
              {isPlaying ? <IconPause size={26} /> : <IconPlay size={26} />}
            </button>
            <button
              className="mobile-miniplayer-btn"
              onClick={() => nextTrack().catch(() => {})}
              aria-label="Next"
            >
              <IconSkipForward size={22} />
            </button>
          </div>
        </div>
        <div
          className="mobile-miniplayer-wave"
          style={{
            paddingTop: 0,
            paddingLeft: 64,
            paddingRight: 14,
          }}
        >
          <div style={{ height: 42 }}>
            <WaveformSeekBar />
          </div>
        </div>
        <button
          className="mobile-miniplayer-art mobile-miniplayer-art-float"
          onClick={onExpand}
          onPointerDown={(e) => e.stopPropagation()}
          aria-label="Open now playing"
          style={{ width: 42, height: 42, top: 68, left: 14 }}
        >
          {artSrc && !artErr ? (
            <img
              src={artSrc}
              alt=""
              crossOrigin="anonymous"
              onLoad={handleArtLoad}
              onError={() => setArtErr(true)}
            />
          ) : (
            <div className="mobile-miniplayer-art-ph">
              <IconMusicNote size={18} />
            </div>
          )}
        </button>
      </div>

      {/* Expanded sheet — always mounted, visibility controlled by CSS */}
      <div
        ref={sheetRef}
        className={`mobile-sheet${expanded ? " expanded" : ""}${dismissing ? " dismissing" : ""}`}
        style={sheetDragY > 0 ? { transform: `translateY(${sheetDragY}px)` } : undefined}
        onTransitionEnd={dismissing ? onSheetTransitionEnd : undefined}
      >
        {sheetBlurColors && (
          <div className="mobile-sheet-bg">
            <UltraBlurBackground colors={sheetBlurColors} />
          </div>
        )}
        <header ref={sheetHeaderRef} className="mobile-sheet-header">
          <div className="mobile-sheet-hint-bar" />
          <button
            className={`mobile-sheet-fav${albumFav ? " active" : ""}`}
            onClick={handleAlbumFavToggle}
            aria-label={albumFav ? "Remove album favourite" : "Favourite album"}
            style={{ top: -6 }}
          >
            {albumFav ? <IconStarFilled /> : <IconStarEmpty />}
          </button>
        </header>
        <div className="mobile-sheet-body" ref={sheetBodyRef}>
          <div
            className="mobile-sheet-main"
            style={mainMinHeight ? { minHeight: mainMinHeight } : undefined}
          >
            <div ref={artRef} className="mobile-sheet-art" style={{ marginBottom: 12 }}>
              {artSrc && !artErr ? (
                <img
                  src={artSrc}
                  alt={track.title}
                  crossOrigin="anonymous"
                  onLoad={handleArtLoad}
                  onError={() => setArtErr(true)}
                  draggable={false}
                />
              ) : (
                <div className="mobile-sheet-art-ph">
                  <IconMusicNote size={64} />
                </div>
              )}
              <LyricsOverlay />
            </div>

            <div className="mobile-sheet-title" style={{ fontSize: 16 }}>
              {track.title}
            </div>
            <div
              className="mobile-sheet-artist"
              role="button"
              tabIndex={0}
              onClick={handleArtistClick}
              style={{ fontSize: 14 }}
            >
              {hasTrackArtist ? `${track.artistName} (${track.trackArtist})` : track.artistName}
            </div>
            {nowPlayingAlbum && (
              <div
                className="mobile-sheet-album"
                role="button"
                tabIndex={0}
                onClick={handleAlbumClick}
                style={{ fontSize: 12 }}
              >
                {nowPlayingAlbum.title}
                {albumYear}
              </div>
            )}

            <div className="mobile-sheet-actions">
              <button
                className={`mobile-sheet-lyrics${showLyrics ? " active" : ""}`}
                onClick={toggleLyrics}
                aria-label={showLyrics ? "Hide lyrics" : "Show lyrics"}
                aria-pressed={showLyrics}
              >
                <IconQuote />
              </button>
              <button
                className="mobile-sheet-eq"
                onClick={() => setShowEQ(true)}
                aria-label="Equalizer"
              >
                <IconEqualizer size={14} />
              </button>
            </div>

            <div
              className="mobile-sheet-wave"
              style={
                {
                  "--sheet-wave-canvas": "50px",
                  "--sheet-time-font": "12px",
                  "--sheet-time-pad": "4px",
                } as React.CSSProperties
              }
            >
              <WaveformSeekBar />
            </div>

            <div className="mobile-sheet-transport">
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => previousTrack().catch(() => {})}
                aria-label="Previous"
              >
                <IconPrevious size={34} />
              </button>
              <button
                className="mobile-sheet-transport-btn primary"
                onClick={() => togglePlayPause().catch(() => {})}
                aria-label={isPlaying ? "Pause" : "Play"}
              >
                {isPlaying ? <IconPause size={56} /> : <IconPlay size={56} />}
              </button>
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => nextTrack().catch(() => {})}
                aria-label="Next"
              >
                <IconNext size={34} />
              </button>
            </div>

            <div className="mobile-sheet-bottom">
              {currentGenres.length > 0 && (
                <div className="mobile-sheet-genres">
                  <FlowLayout genres={currentGenres} onGenreClick={handleGenreClick} />
                </div>
              )}

              <div className="mobile-sheet-foot">
                <span>{codecParts?.label ?? ""}</span>
                <span
                  className={`mobile-sheet-track-fav${trackFav ? " active" : ""}`}
                  onClick={handleTrackFavToggle}
                  role="button"
                  tabIndex={0}
                  aria-label={trackFav ? "Remove track favourite" : "Favourite track"}
                >
                  {trackFav ? <IconStarFilled /> : <IconStarEmpty />}
                </span>
                <span>{codecParts?.detail ?? ""}</span>
              </div>
              {queue.length > queueIndex + 1 && (
                <div className="mobile-sheet-scroll-hint" style={{ paddingTop: 24 }}>
                  <IconChevronDown size={20} />
                </div>
              )}
            </div>
          </div>

          {(() => {
            const upcomingStart = queueIndex + 1;
            const upcoming = queue.slice(upcomingStart);
            if (upcoming.length === 0) return null;
            return (
              <div className="mobile-upnext">
                <div className="mobile-upnext-header">Up Next</div>
                {upcoming.map((t, i) => {
                  const globalIndex = upcomingStart + i;
                  return (
                    <div
                      key={`${globalIndex}-${t.ratingKey}`}
                      className="mobile-upnext-row"
                      role="button"
                      tabIndex={0}
                      onClick={() => jumpToIndex(globalIndex)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          jumpToIndex(globalIndex);
                        }
                      }}
                    >
                      <span className="mobile-upnext-num">{i + 1}</span>
                      <UpNextThumb thumb={t.thumb} />
                      <div className="mobile-upnext-info">
                        <div className="mobile-upnext-title">{t.title}</div>
                        <div className="mobile-upnext-artist">{t.trackArtist || t.artistName}</div>
                      </div>
                      <span className="mobile-upnext-duration">{formatDuration(t.duration)}</span>
                      <button
                        className="mobile-upnext-remove"
                        onClick={(e) => {
                          e.stopPropagation();
                          removeQueueItem(globalIndex);
                        }}
                        aria-label="Remove from queue"
                      >
                        <IconClose size={12} />
                      </button>
                    </div>
                  );
                })}
              </div>
            );
          })()}
        </div>
      </div>
      {showEQ && <EqualizerPanel onDismiss={() => setShowEQ(false)} />}
    </>
  );
}
