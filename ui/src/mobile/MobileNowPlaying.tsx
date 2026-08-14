import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { usePlaybackStore, applyUltraBlurColors } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useGenreInfoStore } from "../stores/genreInfoStore";
import { pushBackHandler } from "../lib/backHandler";
import type { Album } from "../lib/types";
import {
  ART_SIZE,
  setAlbumPalette,
  togglePlayPause,
  nextTrack,
  previousTrack,
  getQueue,
} from "../lib/commands";
import { extractPalette, accentFromPalette } from "../lib/vibrantColor";
import { extractCornerColors } from "../lib/blurArt";
import { applyAccent, DEFAULT_BLUR_COLORS, OLED_VOID_BLUR_COLORS } from "../lib/accent";
import { useArtUrl } from "../lib/useArtUrl";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import { useSheetDrag } from "./useSheetDrag";
import UpNextList from "./UpNextList";
import WaveformSeekBar from "../components/WaveformSeekBar";
import FlowLayout from "../components/FlowLayout";
import UltraBlurBackground from "../components/UltraBlurBackground";
import MarqueeText from "../components/MarqueeText";
import LyricsOverlay from "../components/LyricsOverlay";
import PlaybackQualityNotice from "../components/PlaybackQualityNotice";

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
import CollectionPickerSheet from "./CollectionPickerSheet";
import PlaylistPickerSheet from "./PlaylistPickerSheet";

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

interface Props {
  expanded: boolean;
  onExpand: () => void;
  onCollapse: () => void;
  /** Threaded through to the quality notice, whose only action opens the
      transcode settings when the current mode forbids adapting. */
  onOpenSettings: () => void;
}

/**
 * Mobile now-playing: bottom mini-player when collapsed, full-screen sheet
 * when expanded. Tap the mini-player to expand; chevron-down collapses.
 *
 * Both states share WaveformSeekBar so the cached offscreen shape is
 * rendered once and the progress overlay is cheap. Album-art palette
 * extraction runs once per track on the expanded hero image.
 */
export default function MobileNowPlaying({
  expanded,
  onExpand,
  onCollapse,
  onOpenSettings,
}: Props) {
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
  const moveQueueItem = usePlaybackStore((s) => s.moveQueueItem);
  const clearQueue = usePlaybackStore((s) => s.clearQueue);

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
  // Both pickers resolve their target when the menu row is tapped, never at
  // confirm time. The track and album come from live store selectors, so a
  // natural advance while a picker sits open would silently retarget it —
  // the tap would file whatever is playing by then, not what was chosen.
  const [collectionAlbum, setCollectionAlbum] = useState<Album | null>(null);
  /** null = closed. Carries the snapshot the sheet will commit. */
  const [playlistSheet, setPlaylistSheet] = useState<{
    heading: string;
    ids: string[];
    createOnly?: boolean;
  } | null>(null);

  // --- Swipe gestures ---
  // Pull up from the mini-player to open, pull down from the sheet header or
  // the hero art to dismiss. All of it — including the album-art morph
  // between the two states — lives in useSheetDrag, which drives the sheet
  // imperatively so a drag costs no React renders.
  const miniRef = useRef<HTMLDivElement>(null);
  const miniArtRef = useRef<HTMLButtonElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const sheetBodyRef = useRef<HTMLDivElement>(null);
  const sheetHeaderRef = useRef<HTMLElement>(null);
  const heroArtRef = useRef<HTMLDivElement>(null);

  useSheetDrag(
    {
      sheet: sheetRef,
      body: sheetBodyRef,
      header: sheetHeaderRef,
      heroArt: heroArtRef,
      mini: miniRef,
      miniArt: miniArtRef,
    },
    { expanded, artSrc: artErr ? null : artSrc, onExpand, onCollapse },
  );

  // Entering lyrics mode pins the body (overflow: hidden) — snap any
  // existing scroll offset back to the top so the fixed lyrics layout
  // isn't stuck half-scrolled with the Up Next queue peeking through.
  useEffect(() => {
    if (showLyrics && sheetBodyRef.current) {
      sheetBodyRef.current.scrollTop = 0;
    }
  }, [showLyrics]);

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
      if (collectionAlbum) setCollectionAlbum(null);
      else if (playlistSheet) setPlaylistSheet(null);
      else if (showMenu) setShowMenu(false);
      else onCollapse();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [expanded, onCollapse, showMenu, showEQ, showDebug, collectionAlbum, playlistSheet]);

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
      <div ref={miniRef} className="mobile-miniplayer">
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
            {/* Prefer the track artist: on a compilation the album artist is
                "Various Artists", which says nothing about what is playing. The
                expanded sheet has room for both. */}
            <div className="mobile-miniplayer-artist">{track.trackArtist || track.artistName}</div>
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
          ref={miniArtRef}
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
      <div ref={sheetRef} className={`mobile-sheet${expanded ? " expanded" : ""}`}>
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
            {/* Renders nothing unless the link is struggling, and then the
                art absorbs the row (it's the only flexible item on the
                page). Suppressed in lyrics mode, which regrids this
                container to CSS grid where an extra child would take a
                cell of its own. */}
            {!showLyrics && <PlaybackQualityNotice onOpenSettings={onOpenSettings} />}
            <div ref={heroArtRef} className="mobile-sheet-art">
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

            <MarqueeText className="mobile-sheet-title">{track.title}</MarqueeText>
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

          {hasUpNext && (
            <UpNextList
              queue={queue}
              queueIndex={queueIndex}
              scrollBodyRef={sheetBodyRef}
              onJump={jumpToIndex}
              onRemove={removeQueueItem}
              onMove={moveQueueItem}
            />
          )}
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
                {nowPlayingAlbum && (
                  <button onClick={() => runMenuAction(() => setCollectionAlbum(nowPlayingAlbum))}>
                    Add Album to Collection…
                  </button>
                )}
                {track && (
                  <button
                    onClick={() =>
                      runMenuAction(() =>
                        setPlaylistSheet({ heading: track.title, ids: [track.ratingKey] }),
                      )
                    }
                  >
                    Add Track to Playlist…
                  </button>
                )}
                {queue.length > 0 && (
                  <button
                    onClick={() =>
                      runMenuAction(() =>
                        setPlaylistSheet({
                          heading: "Queue",
                          ids: queue.map((t) => t.ratingKey),
                          createOnly: true,
                        }),
                      )
                    }
                  >
                    Save Queue as Playlist…
                  </button>
                )}
                <button onClick={() => runMenuAction(() => setShowEQ(true))}>Adjust EQ</button>
                <button onClick={() => runMenuAction(() => setShowDebug(true))}>
                  Network Stats for Nerds
                </button>
                <button className="destructive" onClick={() => runMenuAction(clearQueue)}>
                  Clear Queue
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
      {collectionAlbum && (
        <CollectionPickerSheet
          album={collectionAlbum}
          overSheet
          onDismiss={() => setCollectionAlbum(null)}
        />
      )}
      {playlistSheet && (
        <PlaylistPickerSheet
          heading={playlistSheet.heading}
          createOnly={playlistSheet.createOnly}
          getTrackIds={() => Promise.resolve(playlistSheet.ids)}
          overSheet
          onDismiss={() => setPlaylistSheet(null)}
        />
      )}
    </>
  );
}
