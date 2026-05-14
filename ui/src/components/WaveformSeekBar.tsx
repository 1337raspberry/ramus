import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { isHDR } from "../lib/hdr";
import { formatDuration } from "../lib/format";

const UNPLAYED_ALPHA = isHDR ? 0.15 : 0.25;

export default function WaveformSeekBar() {
  const levels = usePlaybackStore((s) => s.waveformLevels);
  const position = usePlaybackStore((s) => s.position);
  const duration = usePlaybackStore((s) => s.duration);
  const seek = usePlaybackStore((s) => s.seek);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [isSeeking, setIsSeeking] = useState(false);
  const [seekPos, setSeekPos] = useState(0);

  // Cache the static waveform shape on an offscreen canvas.
  const shapeCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const lastLevelsRef = useRef<number[] | null>(null);
  const lastSizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 });

  // Bumped by the ResizeObserver on container size change, and listed
  // in both effects' deps so the offscreen shape and progress overlay
  // re-render at the new pixel dimensions. Without this, resizing or
  // entering fullscreen leaves the backing store at the old size and
  // CSS scales it up blurry.
  const [sizeVersion, setSizeVersion] = useState(0);

  const displayPos = isSeeking ? seekPos : position;
  const fraction = duration > 0 ? displayPos / duration : 0;

  // Tracks the AbortController for whichever drag is currently active
  // (mouse OR touch). Stored on a ref so the unmount cleanup can abort
  // it — a track change mid-drag tears down the component while the
  // window listeners are still attached, and without this they'd leak
  // forever with stale closures over `duration` and `seek`.
  const activeDragAbortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    return () => {
      activeDragAbortRef.current?.abort();
      activeDragAbortRef.current = null;
    };
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSizeVersion((v) => v + 1));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;

    // Resize canvas backing store only when dimensions change.
    const needsResize = lastSizeRef.current.w !== w || lastSizeRef.current.h !== h;
    if (needsResize) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      lastSizeRef.current = { w, h };
    }

    if (levels !== lastLevelsRef.current || needsResize) {
      lastLevelsRef.current = levels;
      const offscreen = document.createElement("canvas");
      offscreen.width = w * dpr;
      offscreen.height = h * dpr;
      const ctx = offscreen.getContext("2d")!;
      ctx.scale(dpr, dpr);
      const midY = h / 2;

      if (levels && levels.length > 0) {
        const maxAmp = midY - 4;
        const count = levels.length;
        const stepX = w / count;

        ctx.beginPath();
        let x = 0;
        let amp = levels[0] * maxAmp;
        ctx.moveTo(x, midY - amp);
        for (let i = 1; i < count; i++) {
          x = i * stepX;
          amp = levels[i] * maxAmp;
          const cpX = ((i - 1) * stepX + x) / 2;
          ctx.quadraticCurveTo(cpX, midY - amp, x, midY - amp);
        }
        for (let i = count - 1; i >= 0; i--) {
          x = i * stepX;
          amp = levels[i] * maxAmp;
          if (i === count - 1) {
            ctx.lineTo(x, midY + amp);
          } else {
            const cpX = ((i + 1) * stepX + x) / 2;
            ctx.quadraticCurveTo(cpX, midY + amp, x, midY + amp);
          }
        }
        ctx.closePath();
        // Fill white; colour comes from a later composite pass.
        ctx.fillStyle = "#fff";
        ctx.fill();
      }

      shapeCanvasRef.current = offscreen;
    }
  }, [levels, sizeVersion]);

  // Progress overlay; runs on every position tick.
  useEffect(() => {
    const canvas = canvasRef.current;
    const shape = shapeCanvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = lastSizeRef.current.w;
    const h = lastSizeRef.current.h;
    if (w === 0) return;

    const midY = h / 2;
    const progressX = fraction * w;

    const style = getComputedStyle(document.documentElement);
    const r = style.getPropertyValue("--accent-r").trim();
    const g = style.getPropertyValue("--accent-g").trim();
    const b = style.getPropertyValue("--accent-b").trim();

    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    if (shape && levels && levels.length > 0) {
      // Unplayed silhouette.
      ctx.globalAlpha = UNPLAYED_ALPHA;
      ctx.globalCompositeOperation = "source-over";
      ctx.drawImage(shape, 0, 0, w, h);

      if (progressX > 0) {
        // Played portion, tinted with the accent colour.
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, 0, progressX, h);
        ctx.clip();
        ctx.globalAlpha = 0.9;
        // Mask with the shape, then multiply with the accent.
        ctx.globalCompositeOperation = "source-over";
        ctx.drawImage(shape, 0, 0, w, h);
        ctx.globalCompositeOperation = "multiply";
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.fillRect(0, 0, w, h);
        // Restore alpha from the shape.
        ctx.globalCompositeOperation = "destination-in";
        ctx.drawImage(shape, 0, 0, w, h);
        ctx.restore();
      }

      ctx.globalAlpha = 1;
      ctx.globalCompositeOperation = "source-over";
      ctx.beginPath();
      ctx.moveTo(0, midY);
      ctx.lineTo(w, midY);
      ctx.strokeStyle = "rgba(153, 153, 153, 0.3)";
      ctx.lineWidth = 1;
      ctx.stroke();
    } else {
      ctx.globalAlpha = 1;
      ctx.beginPath();
      ctx.moveTo(0, midY);
      ctx.lineTo(w, midY);
      ctx.strokeStyle = "rgba(153, 153, 153, 0.3)";
      ctx.lineWidth = 2;
      ctx.stroke();

      if (progressX > 0) {
        ctx.beginPath();
        ctx.moveTo(0, midY);
        ctx.lineTo(progressX, midY);
        ctx.strokeStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.lineWidth = 2;
        ctx.stroke();
      }
    }

    ctx.restore();
  }, [levels, fraction, sizeVersion]);

  const handleSeekStart = useCallback(
    (clientX: number) => {
      const el = containerRef.current;
      if (!el || duration <= 0) return;
      const rect = el.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      setIsSeeking(true);
      setSeekPos(frac * duration);
    },
    [duration],
  );

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const el = containerRef.current;
      if (!el || duration <= 0) return;
      handleSeekStart(e.clientX);

      const rect = el.getBoundingClientRect();

      activeDragAbortRef.current?.abort();
      const ac = new AbortController();
      activeDragAbortRef.current = ac;

      const onMove = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        setSeekPos(frac * duration);
      };
      const onUp = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        seek(frac * duration);
        setIsSeeking(false);
        ac.abort();
        if (activeDragAbortRef.current === ac) activeDragAbortRef.current = null;
      };
      window.addEventListener("mousemove", onMove, { signal: ac.signal });
      window.addEventListener("mouseup", onUp, { signal: ac.signal });
    },
    [duration, seek, handleSeekStart],
  );

  const onTouchStart = useCallback(
    (e: React.TouchEvent) => {
      const el = containerRef.current;
      if (!el || duration <= 0) return;
      const touch = e.touches[0];
      if (!touch) return;
      e.stopPropagation();
      handleSeekStart(touch.clientX);

      const rect = el.getBoundingClientRect();

      activeDragAbortRef.current?.abort();
      const ac = new AbortController();
      activeDragAbortRef.current = ac;

      const onMove = (ev: TouchEvent) => {
        const t = ev.touches[0];
        if (!t) return;
        ev.preventDefault();
        const frac = Math.max(0, Math.min(1, (t.clientX - rect.left) / rect.width));
        setSeekPos(frac * duration);
      };
      const onEnd = (ev: TouchEvent) => {
        const t = ev.changedTouches[0];
        if (t) {
          const frac = Math.max(0, Math.min(1, (t.clientX - rect.left) / rect.width));
          seek(frac * duration);
        }
        setIsSeeking(false);
        ac.abort();
        if (activeDragAbortRef.current === ac) activeDragAbortRef.current = null;
      };
      window.addEventListener("touchmove", onMove, { passive: false, signal: ac.signal });
      window.addEventListener("touchend", onEnd, { signal: ac.signal });
      window.addEventListener("touchcancel", onEnd, { signal: ac.signal });
    },
    [duration, seek, handleSeekStart],
  );

  return (
    <div className="waveform-container">
      <div
        ref={containerRef}
        className="waveform-canvas-wrap"
        onMouseDown={onMouseDown}
        onTouchStart={onTouchStart}
      >
        <canvas ref={canvasRef} className="waveform-canvas" />
      </div>
      <div className="waveform-times">
        <span className="waveform-time">{formatDuration(displayPos)}</span>
        <span className="waveform-time">{formatDuration(duration)}</span>
      </div>
    </div>
  );
}
