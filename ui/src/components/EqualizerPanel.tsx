import { useCallback, useEffect, useRef, useState } from "react";
import { applyEqualizer, updateSettings } from "../lib/commands";
import { useSettingsStore } from "../stores/settingsStore";

const BAND_LABELS = ["31", "62", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"];
const MAX_GAIN = 12;

interface Props {
  onDismiss: () => void;
}

function VerticalEQSlider({
  value,
  onChange,
  disabled,
}: {
  value: number;
  onChange: (v: number) => void;
  disabled: boolean;
}) {
  const trackRef = useRef<HTMLDivElement>(null);

  const fraction = (value - -MAX_GAIN) / (MAX_GAIN * 2);
  const thumbPercent = (1 - fraction) * 100;
  const centerPercent = 50;
  const fillTop = Math.min(thumbPercent, centerPercent);
  const fillHeight = Math.abs(thumbPercent - centerPercent);

  const updateFromY = useCallback(
    (clientY: number) => {
      const el = trackRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const frac = 1 - Math.max(0, Math.min(1, (clientY - rect.top) / rect.height));
      const raw = -MAX_GAIN + frac * MAX_GAIN * 2;
      // Snap to 0 when close, 0.5 dB quantization
      const snapped = Math.abs(raw) < 0.8 ? 0 : Math.round(raw * 2) / 2;
      onChange(snapped);
    },
    [onChange],
  );

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (disabled) return;
      e.preventDefault();
      updateFromY(e.clientY);

      const onMove = (ev: MouseEvent) => updateFromY(ev.clientY);
      const onUp = () => {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [disabled, updateFromY],
  );

  return (
    <div
      ref={trackRef}
      className={`eq-slider${disabled ? " disabled" : ""}`}
      onMouseDown={onMouseDown}
    >
      {/* Center line (0 dB) */}
      <div className="eq-center-line" />
      {/* Track */}
      <div className="eq-track" />
      {/* Fill from center to thumb */}
      <div className="eq-fill" style={{ top: `${fillTop}%`, height: `${fillHeight}%` }} />
      {/* Thumb */}
      <div className="eq-thumb" style={{ top: `${thumbPercent}%` }} />
    </div>
  );
}

export default function EqualizerPanel({ onDismiss }: Props) {
  const settings = useSettingsStore();
  const [enabled, setEnabled] = useState(() => settings.eqEnabled);
  const [bands, setBands] = useState<number[]>(() => [
    ...(settings.eqBands ?? new Array(10).fill(0)),
  ]);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const applyDebounced = useCallback((newEnabled: boolean, newBands: number[]) => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      applyEqualizer(newEnabled, newBands).catch(() => {});
      const s = useSettingsStore.getState();
      const updated = { ...s, eqEnabled: newEnabled, eqBands: newBands };
      useSettingsStore.setState(updated);
      updateSettings(updated).catch(() => {});
    }, 50);
  }, []);

  const handleBandChange = useCallback(
    (index: number, value: number) => {
      setBands((prev) => {
        const next = [...prev];
        next[index] = value;
        applyDebounced(enabled, next);
        return next;
      });
    },
    [enabled, applyDebounced],
  );

  const handleToggle = useCallback(() => {
    setEnabled((prev) => {
      const next = !prev;
      applyDebounced(next, bands);
      return next;
    });
  }, [bands, applyDebounced]);

  const handleReset = useCallback(() => {
    const zeroed = new Array(10).fill(0);
    setBands(zeroed);
    applyDebounced(enabled, zeroed);
  }, [enabled, applyDebounced]);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onDismiss]);

  const mouseDownOnBackdrop = useRef(false);

  const handleBackdropMouseDown = useCallback((e: React.MouseEvent) => {
    mouseDownOnBackdrop.current = e.target === e.currentTarget;
  }, []);

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget && mouseDownOnBackdrop.current) onDismiss();
    },
    [onDismiss],
  );

  return (
    <div
      className="eq-backdrop"
      onMouseDown={handleBackdropMouseDown}
      onClick={handleBackdropClick}
    >
      <div className="eq-panel glass">
        {/* Header */}
        <div className="eq-header">
          <span className="eq-title">Equalizer</span>
          <label className="eq-toggle">
            <input type="checkbox" checked={enabled} onChange={handleToggle} />
            <span className="eq-toggle-label">{enabled ? "On" : "Off"}</span>
          </label>
          <button className="eq-close" onClick={onDismiss}>
            x
          </button>
        </div>

        {/* Band sliders */}
        <div className={`eq-bands${!enabled ? " disabled" : ""}`}>
          {/* dB scale */}
          <div className="eq-db-scale">
            <span>+12</span>
            <span>0</span>
            <span>-12</span>
          </div>
          {bands.map((value, i) => (
            <div key={i} className="eq-band-col">
              <VerticalEQSlider
                value={value}
                onChange={(v) => handleBandChange(i, v)}
                disabled={!enabled}
              />
              <span className="eq-band-label">{BAND_LABELS[i]}</span>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="eq-footer">
          <button className="eq-reset" onClick={handleReset}>
            Reset
          </button>
        </div>
      </div>
    </div>
  );
}
