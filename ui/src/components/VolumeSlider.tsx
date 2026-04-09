import { useCallback, useRef, useState } from "react";

interface Props {
  value: number; // 0–100
  onChange: (value: number) => void;
}

export default function VolumeSlider({ value, onChange }: Props) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);

  const fraction = value / 100;

  const updateFromEvent = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      onChange(frac * 100);
    },
    [onChange],
  );

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setDragging(true);
      updateFromEvent(e.clientX);

      const onMove = (ev: MouseEvent) => updateFromEvent(ev.clientX);
      const onUp = () => {
        setDragging(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [updateFromEvent],
  );

  return (
    <div
      ref={trackRef}
      className={`volume-slider${dragging ? " dragging" : ""}`}
      onMouseDown={onMouseDown}
    >
      <div className="volume-track" />
      <div className="volume-fill" style={{ width: `${fraction * 100}%` }} />
      <div className="volume-thumb" style={{ left: `${fraction * 100}%` }} />
    </div>
  );
}
