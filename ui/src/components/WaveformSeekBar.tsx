import { useCallback, useEffect, useRef, useState } from "react";

interface Props {
  levels: number[] | null;
  position: number;
  duration: number;
  bufferedFraction: number;
  isBuffering: boolean;
  onSeek: (seconds: number) => void;
}

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function WaveformSeekBar({
  levels,
  position,
  duration,
  bufferedFraction,
  isBuffering,
  onSeek,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [isSeeking, setIsSeeking] = useState(false);
  const [seekPos, setSeekPos] = useState(0);
  const animRef = useRef<number>(0);

  const displayPos = isSeeking ? seekPos : position;
  const fraction = duration > 0 ? displayPos / duration : 0;

  // Draw waveform
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const midY = h / 2;

    // Read accent color from CSS vars
    const style = getComputedStyle(document.documentElement);
    const r = style.getPropertyValue("--accent-r").trim();
    const g = style.getPropertyValue("--accent-g").trim();
    const b = style.getPropertyValue("--accent-b").trim();

    const progressX = fraction * w;
    const bufferedX = bufferedFraction * w;

    ctx.clearRect(0, 0, w, h);

    if (levels && levels.length > 0) {
      // Build waveform path
      const maxAmp = midY - 4;
      const count = levels.length;
      const stepX = w / count;

      const topPoints: [number, number][] = [];
      const botPoints: [number, number][] = [];
      for (let i = 0; i < count; i++) {
        const x = i * stepX;
        const amp = levels[i] * maxAmp;
        topPoints.push([x, midY - amp]);
        botPoints.push([x, midY + amp]);
      }

      function buildPath(ctx: CanvasRenderingContext2D, top: [number, number][], bot: [number, number][]) {
        ctx.beginPath();
        ctx.moveTo(top[0][0], top[0][1]);
        for (let i = 1; i < top.length; i++) {
          const cpX = (top[i - 1][0] + top[i][0]) / 2;
          ctx.quadraticCurveTo(cpX, top[i][1], top[i][0], top[i][1]);
        }
        for (let i = bot.length - 1; i >= 0; i--) {
          if (i === bot.length - 1) {
            ctx.lineTo(bot[i][0], bot[i][1]);
          } else {
            const cpX = (bot[i + 1][0] + bot[i][0]) / 2;
            ctx.quadraticCurveTo(cpX, bot[i][1], bot[i][0], bot[i][1]);
          }
        }
        ctx.closePath();
      }

      // Unplayed (full waveform, muted)
      buildPath(ctx, topPoints, botPoints);
      ctx.fillStyle = `rgba(153, 153, 153, 0.2)`;
      ctx.fill();

      // Buffered portion
      if (bufferedX > progressX) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(progressX, 0, bufferedX - progressX, h);
        ctx.clip();
        buildPath(ctx, topPoints, botPoints);
        ctx.fillStyle = `rgba(153, 153, 153, 0.35)`;
        ctx.fill();
        ctx.restore();
      }

      // Played portion (accent color)
      if (progressX > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, 0, progressX, h);
        ctx.clip();
        buildPath(ctx, topPoints, botPoints);
        ctx.fillStyle = `rgba(${r}, ${g}, ${b}, 0.85)`;
        ctx.fill();
        ctx.restore();
      }

      // Center line
      ctx.beginPath();
      ctx.moveTo(0, midY);
      ctx.lineTo(w, midY);
      ctx.strokeStyle = `rgba(153, 153, 153, 0.3)`;
      ctx.lineWidth = 1;
      ctx.stroke();
    } else {
      // Thin line fallback
      // Background line
      ctx.beginPath();
      ctx.moveTo(0, midY);
      ctx.lineTo(w, midY);
      ctx.strokeStyle = `rgba(153, 153, 153, 0.3)`;
      ctx.lineWidth = 2;
      ctx.stroke();

      // Buffered line
      if (bufferedX > progressX) {
        ctx.beginPath();
        ctx.moveTo(progressX, midY);
        ctx.lineTo(bufferedX, midY);
        ctx.strokeStyle = `rgba(153, 153, 153, 0.5)`;
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

      // Request next frame for animation
      animRef.current = requestAnimationFrame(() => {
        // Force re-render by dispatching a state-less update
        canvasRef.current?.dispatchEvent(new Event("redraw"));
      });
    }
  }, [levels, fraction, bufferedFraction, isBuffering, duration]);

  // Handle buffering animation loop
  useEffect(() => {
    if (!isBuffering) return;
    let running = true;
    const animate = () => {
      if (!running) return;
      // Trigger canvas redraw
      const canvas = canvasRef.current;
      if (canvas) {
        canvas.dispatchEvent(new Event("redraw"));
      }
      animRef.current = requestAnimationFrame(animate);
    };
    animRef.current = requestAnimationFrame(animate);
    return () => {
      running = false;
      cancelAnimationFrame(animRef.current);
    };
  }, [isBuffering]);

  // Seek via drag
  const handleSeekStart = useCallback(
    (clientX: number) => {
      const el = containerRef.current;
      if (!el || duration <= 0) return;
      const rect = el.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      setIsSeeking(true);
      setSeekPos(frac * duration);
    },
    [duration]
  );

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      handleSeekStart(e.clientX);

      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();

      const onMove = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        setSeekPos(frac * duration);
      };
      const onUp = (ev: MouseEvent) => {
        const frac = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
        onSeek(frac * duration);
        setIsSeeking(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [duration, onSeek, handleSeekStart]
  );

  return (
    <div className="waveform-container">
      <div ref={containerRef} className="waveform-canvas-wrap" onMouseDown={onMouseDown}>
        <canvas ref={canvasRef} className="waveform-canvas" />
      </div>
      <div className="waveform-times">
        <span className="waveform-time">{formatTime(displayPos)}</span>
        <span className="waveform-time">{formatTime(duration)}</span>
      </div>
    </div>
  );
}
