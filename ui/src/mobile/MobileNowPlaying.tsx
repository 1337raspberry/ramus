import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { create } from "zustand";
import { usePlaybackStore } from "../stores/playbackStore";
import {
  ART_SIZE,
  setAlbumPalette,
  togglePlayPause,
  nextTrack,
  previousTrack,
} from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { useArtUrl } from "../lib/useArtUrl";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "../components/WaveformSeekBar";
import FlowLayout from "../components/FlowLayout";
import UltraBlurBackground from "../components/UltraBlurBackground";
import MarqueeText from "../components/MarqueeText";

export const useMiniPlayerDebugStore = create<{
  darken: number;
  wavePadTop: number;
  waveHeight: number;
  wavePadSide: number;
  wavePadLeft: number;
  barPadTop: number;
  barPadBottom: number;
  barPadSide: number;
  barGap: number;
  safeBottom: number;
  artSize: number;
  artTop: number;
  artLeft: number;
  hintTop: number;
  hintWidth: number;
  brickwall: boolean;
  showDebug: boolean;
}>(() => ({
  darken: 0.3,
  wavePadTop: 0,
  waveHeight: 42,
  wavePadSide: 14,
  wavePadLeft: 64,
  barPadTop: 4,
  barPadBottom: 4,
  barPadSide: 14,
  barGap: 0,
  safeBottom: 40,
  artSize: 42,
  artTop: 61,
  artLeft: 14,
  hintTop: 10,
  hintWidth: 50,
  brickwall: false,
  showDebug: false,
}));

const SLIDERS: {
  label: string;
  key: keyof ReturnType<typeof useMiniPlayerDebugStore.getState>;
  min: number;
  max: number;
  scale?: number;
  unit: string;
}[] = [
  { label: "Darken", key: "darken", min: 0, max: 100, scale: 100, unit: "%" },
  { label: "Wave pad top", key: "wavePadTop", min: -60, max: 120, unit: "px" },
  { label: "Wave height", key: "waveHeight", min: 0, max: 240, unit: "px" },
  { label: "Wave pad side", key: "wavePadSide", min: 0, max: 180, unit: "px" },
  { label: "Wave pad left", key: "wavePadLeft", min: -1, max: 360, unit: "px" },
  { label: "Bar pad top", key: "barPadTop", min: -60, max: 120, unit: "px" },
  { label: "Bar pad bottom", key: "barPadBottom", min: -60, max: 120, unit: "px" },
  { label: "Bar pad side", key: "barPadSide", min: 0, max: 180, unit: "px" },
  { label: "Bar gap", key: "barGap", min: 0, max: 120, unit: "px" },
  { label: "Safe bottom", key: "safeBottom", min: -1, max: 150, unit: "px" },
  { label: "Art size", key: "artSize", min: 0, max: 240, unit: "px" },
  { label: "Art top", key: "artTop", min: -180, max: 180, unit: "px" },
  { label: "Art left", key: "artLeft", min: -60, max: 180, unit: "px" },
  { label: "Hint top", key: "hintTop", min: -20, max: 60, unit: "px" },
  { label: "Hint width", key: "hintWidth", min: 10, max: 120, unit: "px" },
];

export function MiniPlayerDebugPanel() {
  const state = useMiniPlayerDebugStore();
  if (!state.showDebug) return null;
  return (
    <div
      style={{
        position: "fixed",
        bottom: 180,
        left: 12,
        right: 12,
        maxHeight: "50vh",
        zIndex: 9999,
        background: "rgba(0,0,0,0.92)",
        borderRadius: 10,
        padding: "8px 14px",
        color: "#fff",
        fontSize: 11,
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div
        style={{
          flex: "1 1 auto",
          overflowY: "auto",
          WebkitOverflowScrolling: "touch",
        }}
      >
        {SLIDERS.map((s) => {
          const scale = s.scale ?? 1;
          const raw = state[s.key] as number;
          const display = Math.round(raw * scale);
          return (
            <div key={s.key} style={{ marginBottom: 2 }}>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span>{s.label}</span>
                <span>
                  {display}
                  {s.unit}
                </span>
              </div>
              <input
                type="range"
                min={s.min}
                max={s.max}
                value={display}
                onChange={(e) =>
                  useMiniPlayerDebugStore.setState({ [s.key]: Number(e.target.value) / scale })
                }
                style={{ width: "100%", height: 20 }}
              />
            </div>
          );
        })}
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 4 }}>
        <button
          style={{
            background: state.brickwall ? "rgba(255,80,80,0.6)" : "rgba(255,255,255,0.15)",
            border: "none",
            borderRadius: 6,
            color: "#fff",
            padding: "4px 10px",
            fontSize: 11,
          }}
          onClick={() => useMiniPlayerDebugStore.setState({ brickwall: !state.brickwall })}
        >
          {state.brickwall ? "Brickwall ON" : "Brickwall"}
        </button>
        <button
          style={{
            background: "rgba(255,255,255,0.15)",
            border: "none",
            borderRadius: 6,
            color: "#fff",
            padding: "4px 10px",
            fontSize: 11,
          }}
          onClick={() => useMiniPlayerDebugStore.setState({ showDebug: false })}
        >
          Close
        </button>
      </div>
    </div>
  );
}
import {
  IconPlay,
  IconPause,
  IconPrevious,
  IconNext,
  IconStarFilled,
  IconStarEmpty,
  IconMusicNote,
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
  const miniDarken = useMiniPlayerDebugStore((s) => s.darken);
  const wavePadTop = useMiniPlayerDebugStore((s) => s.wavePadTop);
  const waveHeight = useMiniPlayerDebugStore((s) => s.waveHeight);
  const barPadTop = useMiniPlayerDebugStore((s) => s.barPadTop);
  const barPadBottom = useMiniPlayerDebugStore((s) => s.barPadBottom);
  const barPadSide = useMiniPlayerDebugStore((s) => s.barPadSide);
  const barGap = useMiniPlayerDebugStore((s) => s.barGap);
  const wavePadSide = useMiniPlayerDebugStore((s) => s.wavePadSide);
  const wavePadLeft = useMiniPlayerDebugStore((s) => s.wavePadLeft);
  const artSize = useMiniPlayerDebugStore((s) => s.artSize);
  const artTop = useMiniPlayerDebugStore((s) => s.artTop);
  const artLeft = useMiniPlayerDebugStore((s) => s.artLeft);
  const safeBottom = useMiniPlayerDebugStore((s) => s.safeBottom);
  const hintTop = useMiniPlayerDebugStore((s) => s.hintTop);
  const hintWidth = useMiniPlayerDebugStore((s) => s.hintWidth);

  const {
    track,
    nowPlayingAlbum,
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
  // threshold. Live transform feedback is intentionally skipped; the CSS
  // keyframe handles the expand animation and a live drag would fight it.
  const SWIPE_THRESHOLD = 50;
  const dragStartY = useRef<number | null>(null);
  const [dragDeltaY, setDragDeltaY] = useState(0);
  const [sheetMounted, setSheetMounted] = useState(expanded);

  useEffect(() => {
    if (expanded) {
      setSheetMounted(true);
    } else if (!sheetDismissing.current) {
      setSheetMounted(false);
    }
  }, [expanded]);

  useLayoutEffect(() => {
    if (!sheetMounted || !expanded) return;
    const el = sheetRef.current;
    if (!el) return;
    const fromY = sheetExpandFrom.current;
    sheetExpandFrom.current = null;
    if (fromY != null) {
      const vh = window.innerHeight;
      const startPct = ((fromY / vh) * 100).toFixed(1);
      el.style.animation = "none";
      el.style.transform = `translateY(${startPct}%)`;
      void el.offsetHeight;
      el.style.transition = "transform 300ms cubic-bezier(0.2, 0.9, 0.2, 1)";
      el.style.transform = "translateY(0)";
      const cleanup = () => {
        el.style.transition = "";
        el.style.transform = "";
      };
      el.addEventListener("transitionend", cleanup, { once: true });
    }
  }, [sheetMounted, expanded]);

  const sheetExpandFrom = useRef<number | null>(null);

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
        sheetExpandFrom.current = e.clientY;
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

  const debugTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onInfoPointerDown = useCallback((_e: React.PointerEvent) => {
    // Debug panel gesture disabled — uncomment to re-enable long-press toggle
    // debugTimer.current = setTimeout(() => {
    //   useMiniPlayerDebugStore.setState((s) => ({ showDebug: !s.showDebug }));
    // }, 800);
    void debugTimer.current;
  }, []);
  const onInfoPointerUp = useCallback(() => {
    if (debugTimer.current) clearTimeout(debugTimer.current);
  }, []);

  const [sheetDragY, setSheetDragY] = useState(0);
  const sheetDismissing = useRef(false);
  const sheetRef = useRef<HTMLDivElement>(null);

  const onSheetHeaderPointerDown = useCallback((e: React.PointerEvent) => {
    if (sheetDismissing.current) return;
    dragStartY.current = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onSheetHeaderPointerMove = useCallback((e: React.PointerEvent) => {
    if (dragStartY.current == null) return;
    const delta = e.clientY - dragStartY.current;
    if (delta > 0) setSheetDragY(delta);
  }, []);

  const onSheetHeaderPointerUp = useCallback(
    (e: React.PointerEvent) => {
      if (dragStartY.current == null) return;
      const delta = e.clientY - dragStartY.current;
      dragStartY.current = null;
      if (delta > SWIPE_THRESHOLD) {
        sheetDismissing.current = true;
        onCollapse();
        const dismiss = () => {
          sheetDismissing.current = false;
          setSheetDragY(0);
          setSheetMounted(false);
        };
        const el = sheetRef.current;
        if (el) {
          el.style.transform = `translateY(${delta}px)`;
          void el.offsetHeight;
          el.style.transition = "transform 200ms cubic-bezier(0.2, 0.9, 0.2, 1)";
          el.style.transform = "translateY(100%)";
          let done = false;
          const finish = () => {
            if (done) return;
            done = true;
            dismiss();
          };
          el.addEventListener("transitionend", finish, { once: true });
          setTimeout(finish, 300);
        } else {
          dismiss();
        }
      } else {
        setSheetDragY(0);
      }
    },
    [onCollapse],
  );

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
        className={`mobile-miniplayer${expanded ? " hidden" : ""}`}
        style={{
          ...(dragDeltaY !== 0 ? { transform: `translateY(${dragDeltaY}px)` } : {}),
          ...(safeBottom >= 0 ? { paddingBottom: safeBottom } : {}),
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
            <div
              className="mobile-miniplayer-darken"
              style={{ background: `rgba(0,0,0,${miniDarken})` }}
            />
          </div>
        )}
        <div className="mobile-miniplayer-hint" style={{ paddingTop: hintTop }}>
          <div className="mobile-miniplayer-hint-pill" style={{ width: hintWidth }} />
        </div>
        <div
          className="mobile-miniplayer-bar"
          style={{ padding: `${barPadTop}px ${barPadSide}px ${barPadBottom}px`, gap: barGap }}
        >
          <div
            className="mobile-miniplayer-info"
            onPointerDown={onInfoPointerDown}
            onPointerUp={onInfoPointerUp}
            onPointerCancel={onInfoPointerUp}
          >
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
            paddingTop: wavePadTop,
            paddingLeft: wavePadLeft >= 0 ? wavePadLeft : wavePadSide,
            paddingRight: wavePadSide,
          }}
        >
          <div style={{ height: waveHeight }}>
            <WaveformSeekBar />
          </div>
        </div>
        <button
          className="mobile-miniplayer-art mobile-miniplayer-art-float"
          onClick={onExpand}
          onPointerDown={(e) => e.stopPropagation()}
          aria-label="Open now playing"
          style={{ width: artSize, height: artSize, top: artTop, left: artLeft }}
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

      {/* Expanded sheet — stays mounted during dismiss animation */}
      {sheetMounted && (
        <div
          ref={sheetRef}
          className="mobile-sheet"
          style={sheetDragY > 0 ? { transform: `translateY(${sheetDragY}px)` } : undefined}
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
            onPointerCancel={onSheetHeaderPointerUp}
          >
            <div className="mobile-sheet-hint-bar" />
            <button
              className={`mobile-sheet-fav${albumFav ? " active" : ""}`}
              onClick={handleAlbumFavToggle}
              aria-label={albumFav ? "Remove album favourite" : "Favourite album"}
            >
              {albumFav ? <IconStarFilled /> : <IconStarEmpty />}
            </button>
          </header>

          <div className="mobile-sheet-body">
            <div className="mobile-sheet-art">
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

            <div className="mobile-sheet-title" onClick={handleAlbumClick}>
              {track.title}
            </div>
            <div className="mobile-sheet-artist" onClick={handleArtistClick}>
              {track.artistName}
            </div>
            {nowPlayingAlbum && (
              <div className="mobile-sheet-album" onClick={handleYearClick}>
                {nowPlayingAlbum.title}
                {albumYear}
              </div>
            )}

            <button
              className="mobile-sheet-lyrics"
              onClick={toggleLyrics}
              aria-label="Toggle lyrics"
            >
              <IconQuote />
            </button>

            <div className="mobile-sheet-wave">
              <WaveformSeekBar />
            </div>

            <div className="mobile-sheet-transport">
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => previousTrack().catch(() => {})}
                aria-label="Previous"
              >
                <IconPrevious />
              </button>
              <button
                className="mobile-sheet-transport-btn primary"
                onClick={() => togglePlayPause().catch(() => {})}
                aria-label={isPlaying ? "Pause" : "Play"}
              >
                {isPlaying ? <IconPause /> : <IconPlay />}
              </button>
              <button
                className="mobile-sheet-transport-btn"
                onClick={() => nextTrack().catch(() => {})}
                aria-label="Next"
              >
                <IconNext />
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
                aria-label={trackFav ? "Remove track favourite" : "Favourite track"}
              >
                {trackFav ? <IconStarFilled /> : <IconStarEmpty />}
              </span>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
