import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { usePlaybackStore, applyUltraBlurColors } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useGenreInfoStore } from "../stores/genreInfoStore";
import { pushBackHandler } from "../lib/backHandler";
import {
  ART_SIZE,
  setAlbumPalette,
  togglePlayPause,
  nextTrack,
  previousTrack,
  getQueue,
} from "../lib/commands";
import { formatDuration } from "../lib/format";
import { extractPalette, accentFromPalette } from "../lib/vibrantColor";
import { extractCornerColors } from "../lib/blurArt";
import { applyAccent, DEFAULT_BLUR_COLORS, OLED_VOID_BLUR_COLORS } from "../lib/accent";
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
  IconMoreDots,
  IconLyrics,
} from "../components/Icons";
import EqualizerPanel from "../components/EqualizerPanel";
import MobileDebugPanel from "./MobileDebugPanel";

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

function UpNextThumb({ thumb }: { thumb: string | null }) {
  const { artSrc: src, artErr: err, setArtErr: setErr } = useArtUrl(thumb, ART_SIZE.SMALL);

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
  const albumBlurColors = usePlaybackStore((s) => s.ultraBlurColors);
  const backgroundStyle = useSettingsStore((s) => s.backgroundStyle);
  const sheetBlurColors = useMemo(() => {
    if (backgroundStyle === "defaultColours") return DEFAULT_BLUR_COLORS;
    if (backgroundStyle === "oledVoid") return OLED_VOID_BLUR_COLORS;
    return albumBlurColors ?? DEFAULT_BLUR_COLORS;
  }, [albumBlurColors, backgroundStyle]);
  const queue = usePlaybackStore((s) => s.queue);
  const queueIndex = usePlaybackStore((s) => s.queueIndex);
  const jumpToIndex = usePlaybackStore((s) => s.jumpToIndex);
  const removeQueueItem = usePlaybackStore((s) => s.removeQueueItem);

  const {
    track,
    nowPlayingAlbum,
    hasTrackArtist,
    year,
    codecBadge,
    albumFav,
    trackFav,
    handleAlbumFavToggle,
    handleTrackFavToggle,
    handleArtistClick,
    handleAlbumClick,
    handleGenreClick,
  } = useNowPlayingActions({ onNavigate: onCollapse });
  const openGenreInfo = useGenreInfoStore((s) => s.open);

  const thumb = track?.thumb ?? nowPlayingAlbum?.thumb ?? null;
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.LARGE);
  const lastAccentThumb = useRef<string | null>(null);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      const img = e.currentTarget;
      if (lastAccentThumb.current === thumb) return;
      lastAccentThumb.current = thumb;
      const capturedThumb = thumb;
      // Art-derived corner colours override the server-provided instant
      // paint the moment the art decodes (spatial extraction, see
      // lib/blurArt.ts). Independent of the palette cache below, which
      // only feeds the accent.
      const corners = extractCornerColors(img);
      if (corners) applyUltraBlurColors(corners, "extracted");
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
        // Palette feeds the accent + DB cache only; the UltraBlur corners
        // come from the server-provided colours via getAlbumColors.
        usePlaybackStore.setState({ vibrantPalette: palette });
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
  const [showDebug, setShowDebug] = useState(false);
  const [showMenu, setShowMenu] = useState(false);

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
  // The drag-dismiss path sets `dismissing=true` and waits for the
  // `transform` transitionend to fire `onCollapse()` and reset the flag.
  // If `expanded` flips to false by an external path first (Android
  // hardware back collapses the sheet directly), the transitionend may
  // never arrive and `dismissing` stays stuck. Next open then renders the
  // sheet with `dismissing=true`, transitionend fires on the open
  // animation, and the sheet immediately re-collapses. Resetting on every
  // collapse keeps the flag consistent with the rendered state.
  useEffect(() => {
    if (!expanded) setDismissing(false);
  }, [expanded]);
  const sheetRef = useRef<HTMLDivElement>(null);
  const sheetBodyRef = useRef<HTMLDivElement>(null);
  const sheetHeaderRef = useRef<HTMLElement>(null);
  const sheetDragYRef = useRef(0);

  // Entering lyrics mode pins the body (overflow: hidden) — snap any
  // existing scroll offset back to the top so the fixed lyrics layout
  // isn't stuck half-scrolled with the Up Next queue peeking through.
  useEffect(() => {
    if (showLyrics && sheetBodyRef.current) {
      sheetBodyRef.current.scrollTop = 0;
    }
  }, [showLyrics]);

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

  // Nudge the body down far enough to reveal the Up Next header, then
  // spring back — a hint that there is more below rather than a jump.
  const peekUpNext = useCallback(() => {
    const el = sheetBodyRef.current;
    if (!el) return;
    el.scrollTo({ top: 90, behavior: "smooth" });
    window.setTimeout(() => {
      el.scrollTo({ top: 0, behavior: "smooth" });
    }, 450);
  }, []);

  // Menu rows dismiss first so the action lands on a settled UI (the
  // navigation rows also collapse the sheet out from under the menu).
  const runMenuAction = useCallback((action: () => void) => {
    setShowMenu(false);
    action();
  }, []);

  // Close the sheet on Escape (iOS keyboard / external keyboard). The
  // overflow menu is nested inside the sheet, so it consumes Escape first.
  // The EQ and debug panels handle their own Escape, and this listener is
  // on `window` so it would fire alongside theirs — yield while either is
  // open, or one keypress closes the panel AND collapses the sheet.
  useEffect(() => {
    if (!expanded) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (showEQ || showDebug) return;
      if (showMenu) setShowMenu(false);
      else onCollapse();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [expanded, onCollapse, showMenu, showEQ, showDebug]);

  // Same for hardware back — without this the sheet collapses out from
  // under the menu, stranding it (it portals to <body>, so it does not
  // unmount with the sheet).
  useEffect(() => {
    if (!showMenu) return;
    return pushBackHandler(() => {
      setShowMenu(false);
      return true;
    });
  }, [showMenu]);

  if (!track) return null;

  const isPlaying = status === "playing";
  const albumYear = year ? ` (${year})` : "";
  const hasUpNext = queue.length > queueIndex + 1;

  return (
    <>
      {/* Mini-player: always mounted to keep the waveform offscreen shape
          warm, hidden when expanded so taps hit the sheet. */}
      <div
        ref={miniRef}
        className="mobile-miniplayer"
        style={dragDeltaY !== 0 ? { transform: `translateY(${dragDeltaY}px)` } : undefined}
      >
        <div className="mobile-miniplayer-bg">
          <UltraBlurBackground colors={sheetBlurColors} />
          <div className="mobile-miniplayer-darken" style={{ background: "rgba(0,0,0,0.3)" }} />
        </div>
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
        <div className="mobile-sheet-bg">
          <UltraBlurBackground colors={sheetBlurColors} />
        </div>
        <header ref={sheetHeaderRef} className="mobile-sheet-header">
          <div className="mobile-sheet-hint-bar" />
        </header>
        <div
          className={`mobile-sheet-body${showLyrics ? " lyrics-active" : ""}`}
          ref={sheetBodyRef}
        >
          <div className={`mobile-sheet-main${showLyrics ? " lyrics-mode" : ""}`}>
            <div ref={artRef} className="mobile-sheet-art">
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
            </div>

            {/* Sibling of the art box (not inside it): lyrics mode turns
                the main container into a grid where the art shrinks to a
                thumbnail and the overlay takes the whole middle row. The
                art <img> stays mounted so palette extraction keeps
                working across track changes while lyrics are open. */}
            <LyricsOverlay />

            <div className="mobile-sheet-title">{track.title}</div>
            <div className="mobile-sheet-artist">
              {hasTrackArtist ? `${track.artistName} (${track.trackArtist})` : track.artistName}
            </div>
            {showLyrics && (
              <button
                className="mobile-lyrics-exit"
                onClick={toggleLyrics}
                aria-label="Hide lyrics"
              >
                <IconClose size={16} />
              </button>
            )}
            {nowPlayingAlbum && (
              <div className="mobile-sheet-album">
                {nowPlayingAlbum.title}
                {albumYear}
              </div>
            )}

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
                className={`mobile-sheet-transport-btn secondary${showLyrics ? " active" : ""}`}
                onClick={toggleLyrics}
                aria-label={showLyrics ? "Hide lyrics" : "Show lyrics"}
                aria-pressed={showLyrics}
              >
                <IconLyrics size={24} />
              </button>
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
              <button
                className={`mobile-sheet-transport-btn secondary${trackFav ? " active" : ""}`}
                onClick={handleTrackFavToggle}
                aria-label={trackFav ? "Remove track favourite" : "Favourite track"}
                aria-pressed={trackFav}
              >
                {trackFav ? <IconStarFilled size={22} /> : <IconStarEmpty size={22} />}
              </button>
            </div>

            <div className="mobile-sheet-bottom">
              {currentGenres.length > 0 && (
                <div className="mobile-sheet-genres">
                  <FlowLayout
                    genres={currentGenres}
                    onGenreClick={handleGenreClick}
                    onGenreLongPress={openGenreInfo}
                  />
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

        {/* Pinned dock — a sibling of the scroll body, so its contents stay
            on screen no matter how tall the track's genre list grows. */}
        <div className="mobile-sheet-dock">
          {hasUpNext && !showLyrics ? (
            <button
              type="button"
              className="mobile-sheet-dock-btn hint"
              onClick={peekUpNext}
              aria-label="Show up next"
            >
              <IconChevronDown size={22} />
            </button>
          ) : (
            /* Holds the chevron's slot so the menu button keeps its
               position when there is nothing queued after this track. */
            <span className="mobile-sheet-dock-spacer" aria-hidden="true" />
          )}
          {codecBadge && <span className="mobile-sheet-badge">{codecBadge}</span>}
          <button
            type="button"
            className="mobile-sheet-dock-btn"
            onClick={() => setShowMenu(true)}
            aria-label="More actions"
            aria-haspopup="menu"
          >
            <IconMoreDots size={22} />
          </button>
        </div>
      </div>
      {showMenu &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop over-sheet"
            onClick={(e) => {
              if (e.target === e.currentTarget) setShowMenu(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <button onClick={() => runMenuAction(handleArtistClick)}>Go to Artist</button>
                {nowPlayingAlbum && (
                  <button onClick={() => runMenuAction(handleAlbumClick)}>Go to Album</button>
                )}
                {nowPlayingAlbum && (
                  <button onClick={() => runMenuAction(handleAlbumFavToggle)}>
                    <span className="mobile-action-sheet-icon">
                      {albumFav ? <IconStarFilled size={20} /> : <IconStarEmpty size={20} />}
                    </span>
                    {albumFav ? "Remove Album Favourite" : "Favourite Album"}
                  </button>
                )}
                <button onClick={() => runMenuAction(() => setShowEQ(true))}>Adjust EQ</button>
                <button onClick={() => runMenuAction(() => setShowDebug(true))}>
                  Network Stats for Nerds
                </button>
              </div>
              <button className="mobile-action-sheet-cancel" onClick={() => setShowMenu(false)}>
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}
      {showEQ && <EqualizerPanel onDismiss={() => setShowEQ(false)} />}
      {showDebug && <MobileDebugPanel onDismiss={() => setShowDebug(false)} />}
    </>
  );
}
