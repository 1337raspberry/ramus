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
import { useArtUrl } from "../lib/useArtUrl";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "../components/WaveformSeekBar";
import FlowLayout from "../components/FlowLayout";
import UltraBlurBackground from "../components/UltraBlurBackground";
import MarqueeText from "../components/MarqueeText";

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
} from "../components/Icons";

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
    codec,
    albumFav,
    trackFav,
    handleAlbumFavToggle,
    handleTrackFavToggle,
    handleArtistClick,
    handleAlbumClick,
    handleYearClick,
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
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        return;
      }
      extractPalette(img).then((palette) => {
        if (!palette || lastAccentThumb.current !== capturedThumb) return;
        const [r, g, b] = accentFromPalette(palette);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        const blurColors = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blurColors });
        if (track?.albumKey) {
          setAlbumPalette(track.albumKey, palette).catch(() => {});
        }
      });
    },
    [thumb, track?.albumKey],
  );

  // Closed-captions-style lyrics button (not yet wired to anything on mobile).
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);

  // --- Swipe gestures ---
  // Mini-player: swipe up to expand. Sheet header: swipe down to collapse.
  // We track pointer delta-Y and commit on pointerup when it crosses a
  // threshold. The sheet is always mounted — expand/collapse just toggles
  // the .expanded CSS class which drives a transform transition.
  const SWIPE_THRESHOLD = 50;
  const dragStartY = useRef<number | null>(null);
  const [dragDeltaY, setDragDeltaY] = useState(0);

  const onMiniPointerDown = useCallback((e: React.PointerEvent) => {
    dragStartY.current = e.clientY;
    setDragDeltaY(0);
  }, []);

  const onMiniPointerMove = useCallback((e: React.PointerEvent) => {
    if (dragStartY.current == null) return;
    const delta = e.clientY - dragStartY.current;
    if (delta < 0) setDragDeltaY(delta);
  }, []);

  const onMiniPointerUp = useCallback(
    (e: React.PointerEvent) => {
      if (dragStartY.current == null) return;
      const delta = e.clientY - dragStartY.current;
      dragStartY.current = null;
      setDragDeltaY(0);
      if (delta < -SWIPE_THRESHOLD) {
        onExpand();
      }
    },
    [onExpand],
  );

  // Stops a pointerdown inside a child (waveform, controls) from being
  // seen by the outer drag tracker. Without this, a touch anywhere in
  // the mini-player records dragStartY and the pointerup logic can't
  // distinguish "scrub finished at same Y" from "tap anywhere".
  const swallowPointerDown = useCallback((e: React.PointerEvent) => {
    e.stopPropagation();
  }, []);

  const [sheetDragY, setSheetDragY] = useState(0);
  const [dismissing, setDismissing] = useState(false);
  const sheetRef = useRef<HTMLDivElement>(null);
  const sheetBodyRef = useRef<HTMLDivElement>(null);
  const [mainMinHeight, setMainMinHeight] = useState<number | undefined>(undefined);

  useEffect(() => {
    if (!expanded) return;
    const el = sheetBodyRef.current;
    if (el) setMainMinHeight(el.clientHeight);
  }, [expanded, track?.ratingKey]);

  const onSheetHeaderPointerDown = useCallback((e: React.PointerEvent) => {
    dragStartY.current = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onSheetHeaderPointerMove = useCallback((e: React.PointerEvent) => {
    if (dragStartY.current == null) return;
    const delta = e.clientY - dragStartY.current;
    if (delta > 0) setSheetDragY(delta);
  }, []);

  const onSheetHeaderPointerUp = useCallback((e: React.PointerEvent) => {
    if (dragStartY.current == null) return;
    const delta = e.clientY - dragStartY.current;
    dragStartY.current = null;
    if (delta > SWIPE_THRESHOLD) {
      setSheetDragY(0);
      setDismissing(true);
    } else {
      setSheetDragY(0);
    }
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
        className="mobile-miniplayer"
        style={{
          ...(dragDeltaY !== 0 ? { transform: `translateY(${dragDeltaY}px)` } : {}),
          paddingBottom: 40,
        }}
        onPointerDown={onMiniPointerDown}
        onPointerMove={onMiniPointerMove}
        onPointerUp={onMiniPointerUp}
        onPointerCancel={() => {
          dragStartY.current = null;
          setDragDeltaY(0);
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
          onPointerDown={swallowPointerDown}
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
        <header
          className="mobile-sheet-header"
          onPointerDown={onSheetHeaderPointerDown}
          onPointerMove={onSheetHeaderPointerMove}
          onPointerUp={onSheetHeaderPointerUp}
          onPointerCancel={() => {
            dragStartY.current = null;
            setSheetDragY(0);
          }}
        >
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
            <div className="mobile-sheet-art" style={{ marginBottom: 12 }}>
              {artSrc && !artErr ? (
                <img
                  src={artSrc}
                  alt={track.title}
                  crossOrigin="anonymous"
                  onLoad={handleArtLoad}
                  onError={() => setArtErr(true)}
                />
              ) : (
                <div className="mobile-sheet-art-ph">
                  <IconMusicNote size={64} />
                </div>
              )}
            </div>

            <div
              className="mobile-sheet-title"
              role="button"
              tabIndex={0}
              onClick={handleAlbumClick}
              style={{ fontSize: 16 }}
            >
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
                onClick={handleYearClick}
                style={{ fontSize: 12 }}
              >
                {nowPlayingAlbum.title}
                {albumYear}
              </div>
            )}

            <button
              className="mobile-sheet-lyrics"
              onClick={toggleLyrics}
              aria-label="Toggle lyrics"
              style={{
                width: 40,
                height: 18,
                marginTop: 8,
              }}
            >
              <IconQuote />
            </button>

            <div
              className="mobile-sheet-wave"
              style={
                {
                  height: 60,
                  marginTop: 10,
                  "--sheet-wave-canvas": "50px",
                  "--sheet-time-font": "12px",
                  "--sheet-time-pad": "4px",
                } as React.CSSProperties
              }
            >
              <WaveformSeekBar />
            </div>

            <div className="mobile-sheet-transport" style={{ gap: 42 }}>
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => previousTrack().catch(() => {})}
                aria-label="Previous"
                style={{ width: 48, height: 48 }}
              >
                <IconPrevious size={34} />
              </button>
              <button
                className="mobile-sheet-transport-btn primary"
                onClick={() => togglePlayPause().catch(() => {})}
                aria-label={isPlaying ? "Pause" : "Play"}
                style={{ width: 48, height: 48 }}
              >
                {isPlaying ? <IconPause size={56} /> : <IconPlay size={56} />}
              </button>
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => nextTrack().catch(() => {})}
                aria-label="Next"
                style={{ width: 48, height: 48 }}
              >
                <IconNext size={34} />
              </button>
            </div>

            {currentGenres.length > 0 && (
              <div className="mobile-sheet-genres">
                <FlowLayout genres={currentGenres} onGenreClick={handleGenreClick} />
              </div>
            )}

            <div className="mobile-sheet-foot">
              {codec && <span>{codec}</span>}
              <span
                className={`mobile-sheet-track-fav${trackFav ? " active" : ""}`}
                onClick={handleTrackFavToggle}
                role="button"
                tabIndex={0}
                aria-label={trackFav ? "Remove track favourite" : "Favourite track"}
              >
                {trackFav ? <IconStarFilled /> : <IconStarEmpty />}
              </span>
            </div>
            {queue.length > queueIndex + 1 && (
              <div className="mobile-sheet-scroll-hint" style={{ paddingTop: 44 }}>
                <IconChevronDown size={20} />
              </div>
            )}
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
    </>
  );
}
