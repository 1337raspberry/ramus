import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { formatDuration } from "../lib/format";

export default function WaveformSeekBar() {
  const levels = usePlaybackStore((s) => s.waveformLevels);
  const position = usePlaybackStore((s) => s.position);
  const duration = usePlaybackStore((s) => s.duration);
  const bufferedFraction = usePlaybackStore((s) => s.bufferedFraction);
  const isBuffering = usePlaybackStore((s) => s.isBuffering);
  const seek = usePlaybackStore((s) => s.seek);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [isSeeking, setIsSeeking] = useState(false);
  const [seekPos, setSeekPos] = useState(0);
  const animRef = useRef<number>(0);

  // Cache the static waveform shape on an offscreen canvas
  const shapeCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const lastLevelsRef = useRef<number[] | null>(null);
  const lastSizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 });

  const displayPos = isSeeking ? seekPos : position;
  const fraction = duration > 0 ? displayPos / duration : 0;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;

    // Resize canvas backing store only when dimensions change
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
        // Fill with white — we'll composite with colors later
        ctx.fillStyle = "#fff";
        ctx.fill();
      }

      shapeCanvasRef.current = offscreen;
    }
  }, [levels]);

  // Draw progress overlay (runs on every position tick; inexpensive)
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
    const bufferedX = bufferedFraction * w;

    const style = getComputedStyle(document.documentElement);
    const r = style.getPropertyValue("--accent-r").trim();
    const g = style.getPropertyValue("--accent-g").trim();
    const b = style.getPropertyValue("--accent-b").trim();

    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    if (shape && levels && levels.length > 0) {
      // Unplayed
      ctx.globalAlpha = 0.15;
      ctx.globalCompositeOperation = "source-over";
      ctx.drawImage(shape, 0, 0, w, h);

      // Played portion (accent color)
      if (progressX > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, 0, progressX, h);
        ctx.clip();
        ctx.globalAlpha = 0.9;
        // Draw shape as mask, then tint
        ctx.globalCompositeOperation = "source-over";
        ctx.drawImage(shape, 0, 0, w, h);
        // Multiply with accent color
        ctx.globalCompositeOperation = "multiply";
        ctx.fillStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.fillRect(0, 0, w, h);
        // Restore alpha from shape
        ctx.globalCompositeOperation = "destination-in";
        ctx.drawImage(shape, 0, 0, w, h);
        ctx.restore();
      }

      // Center line
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
      // Background line
      ctx.beginPath();
      ctx.moveTo(0, midY);
      ctx.lineTo(w, midY);
      ctx.strokeStyle = "rgba(153, 153, 153, 0.3)";
      ctx.lineWidth = 2;
      ctx.stroke();

      // Buffered line
      if (bufferedX > progressX) {
        ctx.beginPath();
        ctx.moveTo(progressX, midY);
        ctx.lineTo(bufferedX, midY);
        ctx.strokeStyle = "rgba(153, 153, 153, 0.5)";
        ctx.lineWidth = 2;
        ctx.stroke();
      }

      // Progress line
      if (progressX > 0) {
        ctx.beginPath();
        ctx.moveTo(0, midY);
        ctx.lineTo(progressX, midY);
        ctx.strokeStyle = `rgb(${r}, ${g}, ${b})`;
        ctx.lineWidth = 2;
        ctx.stroke();
      }
    }

    // Buffering scan animation
    if (isBuffering) {
      const scanWidth = w * 0.18;
      const t = (performance.now() % 1400) / 1400;
      const scanX = t * (w - scanWidth);
      const gradient = ctx.createLinearGradient(scanX, 0, scanX + scanWidth, 0);
      gradient.addColorStop(0, "transparent");
      gradient.addColorStop(0.5, `rgba(${r}, ${g}, ${b}, 0.45)`);
      gradient.addColorStop(1, "transparent");
      ctx.fillStyle = gradient;
      ctx.fillRect(scanX, 0, scanWidth, h);
    }

    ctx.restore();
  }, [levels, fraction, bufferedFraction, isBuffering]);

  useEffect(() => {
    if (!isBuffering) return;
    let running = true;
    const animate = () => {
      if (!running) return;
      // Repaint via requestAnimationFrame with direct canvas rendering
      const canvas = canvasRef.current;
      if (canvas) {
        animRef.current = requestAnimationFrame(animate);
      }
    };
    animRef.current = requestAnimationFrame(animate);
    return () => {
      running = false;
      cancelAnimationFrame(animRef.current);
    };
  }, [isBuffering]);

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

      const onMove = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        setSeekPos(frac * duration);
      };
      const onUp = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        seek(frac * duration);
        setIsSeeking(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [duration, seek, handleSeekStart],
  );

  return (
    <div className="waveform-container">
      <div ref={containerRef} className="waveform-canvas-wrap" onMouseDown={onMouseDown}>
        <canvas ref={canvasRef} className="waveform-canvas" />
      </div>
      <div className="waveform-times">
        <span className="waveform-time">{formatDuration(displayPos)}</span>
        <span className="waveform-time">{formatDuration(duration)}</span>
      </div>
    </div>
  );
}
