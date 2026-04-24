import { useCallback, useEffect, useRef, useState } from "react";
import { applyEqualizer, updateSettings, getEqConfig, type EqConfig } from "../lib/commands";
import { useSettingsStore } from "../stores/settingsStore";

const DEFAULT_LABELS = ["31", "62", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"];
const DEFAULT_MAX_GAIN = 12;

function formatFreqLabel(hz: number): string {
  return hz >= 1000 ? `${hz / 1000}K` : String(hz);
}

interface Props {
  onDismiss: () => void;
}

function VerticalEQSlider({
  value,
  onChange,
  disabled,
  maxGain,
}: {
  value: number;
  onChange: (v: number) => void;
  disabled: boolean;
  maxGain: number;
}) {
  const trackRef = useRef<HTMLDivElement>(null);

  const fraction = (value - -maxGain) / (maxGain * 2);
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
      const raw = -maxGain + frac * maxGain * 2;
      const snapped = Math.abs(raw) < 0.8 ? 0 : Math.round(raw * 2) / 2;
      onChange(snapped);
    },
    [onChange, maxGain],
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

  useEffect(() => {
    const el = trackRef.current;
    if (!el || disabled) return;

    const onTouchStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) return;
      e.preventDefault();
      updateFromY(e.touches[0].clientY);

      const onTouchMove = (ev: TouchEvent) => {
        ev.preventDefault();
        if (ev.touches.length === 1) updateFromY(ev.touches[0].clientY);
      };
      const onTouchEnd = () => {
        window.removeEventListener("touchmove", onTouchMove);
        window.removeEventListener("touchend", onTouchEnd);
        window.removeEventListener("touchcancel", onTouchEnd);
      };
      window.addEventListener("touchmove", onTouchMove, { passive: false });
      window.addEventListener("touchend", onTouchEnd);
      window.addEventListener("touchcancel", onTouchEnd);
    };

    el.addEventListener("touchstart", onTouchStart, { passive: false });
    return () => el.removeEventListener("touchstart", onTouchStart);
  }, [disabled, updateFromY]);

  return (
    <div
      ref={trackRef}
      className={`eq-slider${disabled ? " disabled" : ""}`}
      onMouseDown={onMouseDown}
    >
      <div className="eq-center-line" />
      <div className="eq-track" />
      <div className="eq-fill" style={{ top: `${fillTop}%`, height: `${fillHeight}%` }} />
      <div className="eq-thumb" style={{ top: `${thumbPercent}%` }} />
    </div>
  );
}

export default function EqualizerPanel({ onDismiss }: Props) {
  const settings = useSettingsStore();
  const [eqConfig, setEqConfig] = useState<EqConfig | null>(null);
  const [enabled, setEnabled] = useState(() => settings.eqEnabled);
  const [bands, setBands] = useState<number[]>(() => [
    ...(settings.eqBands ?? new Array(10).fill(0)),
  ]);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    getEqConfig()
      .then((config) => {
        setEqConfig(config);
        setBands((prev) => {
          if (prev.length === config.frequencies.length) return prev;
          const next = new Array(config.frequencies.length).fill(0);
          for (let i = 0; i < Math.min(prev.length, next.length); i++) next[i] = prev[i];
          return next;
        });
      })
      .catch(() => {});
  }, []);

  const bandCount = eqConfig?.frequencies.length ?? bands.length;
  const maxGain = eqConfig?.maxGain ?? DEFAULT_MAX_GAIN;
  const bandLabels = eqConfig ? eqConfig.frequencies.map(formatFreqLabel) : DEFAULT_LABELS;

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
    const zeroed = new Array(bandCount).fill(0);
    setBands(zeroed);
    applyDebounced(enabled, zeroed);
  }, [enabled, applyDebounced, bandCount]);

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

        <div className={`eq-bands${!enabled ? " disabled" : ""}`}>
          <div className="eq-db-scale">
            <span>+{Math.round(maxGain)}</span>
            <span>0</span>
            <span>-{Math.round(maxGain)}</span>
          </div>
          {bands.map((value, i) => (
            <div key={i} className="eq-band-col">
              <VerticalEQSlider
                value={value}
                onChange={(v) => handleBandChange(i, v)}
                disabled={!enabled}
                maxGain={maxGain}
              />
              <span className="eq-band-label">{bandLabels[i]}</span>
            </div>
          ))}
        </div>

        <div className="eq-footer">
          <button className="eq-reset" onClick={handleReset}>
            Reset
          </button>
        </div>
      </div>
    </div>
  );
}
