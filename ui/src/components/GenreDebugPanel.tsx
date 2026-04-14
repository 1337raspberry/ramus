import { create } from "zustand";

// Shared store so GenreTreeView can subscribe to these values.
export const useGenreDebugStore = create<{
  chevronSize: number;
  chevronWidth: number;
  textSize: number;
  padH: number;
  rowHeight: number;
  indentDepth: number;
}>(() => ({
  chevronSize: 18,
  chevronWidth: 20,
  textSize: 12,
  padH: 6,
  rowHeight: 30,
  indentDepth: 8,
}));

const SLIDERS = [
  { label: "Chevron size", key: "chevronSize" as const, min: 8, max: 28 },
  { label: "Chevron width", key: "chevronWidth" as const, min: 8, max: 40 },
  { label: "Text size", key: "textSize" as const, min: 10, max: 24 },
  { label: "Pad H", key: "padH" as const, min: 0, max: 24 },
  { label: "Row height", key: "rowHeight" as const, min: 16, max: 48 },
  { label: "Indent depth", key: "indentDepth" as const, min: 4, max: 32 },
];

export default function GenreDebugPanel() {
  const state = useGenreDebugStore();

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
        minWidth: 260,
      }}
    >
      <div style={{ fontWeight: 700, marginBottom: 2 }}>Genre Tree Debug</div>
      {SLIDERS.map((s) => (
        <div key={s.key} style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ width: 100, flexShrink: 0 }}>{s.label}</span>
          <input
            type="range"
            min={s.min}
            max={s.max}
            step={1}
            value={state[s.key]}
            onChange={(e) => useGenreDebugStore.setState({ [s.key]: Number(e.target.value) })}
            style={{ flex: 1 }}
          />
          <span style={{ width: 36, textAlign: "right" }}>{state[s.key]}px</span>
        </div>
      ))}
    </div>
  );
}
