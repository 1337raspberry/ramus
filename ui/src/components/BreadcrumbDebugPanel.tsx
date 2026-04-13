import { create } from "zustand";

export const useBreadcrumbDebugStore = create<{
  fadeHeight: number;
  fadeStartOpacity: number;
  fontSize: number;
  crumbPadH: number;
  crumbPadV: number;
  crumbGap: number;
  barPadH: number;
  barPadV: number;
  sepSpacing: number;
}>(() => ({
  fadeHeight: 12,
  fadeStartOpacity: 0.03,
  fontSize: 11,
  crumbPadH: 0,
  crumbPadV: 0,
  crumbGap: 4,
  barPadH: 20,
  barPadV: 13,
  sepSpacing: 2,
}));

const SLIDERS = [
  { label: "Fade height", key: "fadeHeight" as const, min: 0, max: 120, unit: "px" },
  {
    label: "Fade start α",
    key: "fadeStartOpacity" as const,
    min: 0,
    max: 100,
    unit: "%",
    scale: 100,
  },
  { label: "Font size", key: "fontSize" as const, min: 8, max: 18, unit: "px" },
  { label: "Crumb pad H", key: "crumbPadH" as const, min: 0, max: 16, unit: "px" },
  { label: "Crumb pad V", key: "crumbPadV" as const, min: 0, max: 8, unit: "px" },
  { label: "Crumb gap", key: "crumbGap" as const, min: 0, max: 16, unit: "px" },
  { label: "Bar pad H", key: "barPadH" as const, min: 0, max: 24, unit: "px" },
  { label: "Bar pad V", key: "barPadV" as const, min: 0, max: 16, unit: "px" },
  { label: "Sep spacing", key: "sepSpacing" as const, min: 0, max: 12, unit: "px" },
] as const;

export default function BreadcrumbDebugPanel() {
  const state = useBreadcrumbDebugStore();

  return (
    <div
      style={{
        position: "fixed",
        top: 40,
        right: 12,
        zIndex: 99999,
        background: "rgba(0,0,0,0.92)",
        border: "1px solid #555",
        borderRadius: 8,
        padding: "10px 14px",
        display: "flex",
        flexDirection: "column",
        gap: 6,
        fontFamily: "monospace",
        fontSize: 11,
        color: "#ccc",
        minWidth: 280,
      }}
    >
      <div style={{ fontWeight: 700, marginBottom: 2 }}>Breadcrumb Debug</div>
      {SLIDERS.map((s) => {
        const scale = "scale" in s ? s.scale : 1;
        const displayVal = Math.round(state[s.key] * scale);
        return (
          <div key={s.key} style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ width: 100, flexShrink: 0 }}>{s.label}</span>
            <input
              type="range"
              min={s.min}
              max={s.max}
              step={1}
              value={displayVal}
              onChange={(e) =>
                useBreadcrumbDebugStore.setState({ [s.key]: Number(e.target.value) / scale })
              }
              style={{ flex: 1 }}
            />
            <span style={{ width: 40, textAlign: "right" }}>
              {displayVal}
              {s.unit}
            </span>
          </div>
        );
      })}
    </div>
  );
}
