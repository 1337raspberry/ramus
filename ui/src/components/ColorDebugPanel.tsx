import { useEffect, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";

export default function ColorDebugPanel() {
  const ultraBlurColors = usePlaybackStore((s) => s.ultraBlurColors);
  const [accent, setAccent] = useState({ r: 0, g: 0, b: 0 });

  useEffect(() => {
    const poll = () => {
      const s = document.documentElement.style;
      setAccent({
        r: parseInt(s.getPropertyValue("--accent-r")) || 0,
        g: parseInt(s.getPropertyValue("--accent-g")) || 0,
        b: parseInt(s.getPropertyValue("--accent-b")) || 0,
      });
    };
    poll();
    const id = setInterval(poll, 500);
    return () => clearInterval(id);
  }, []);

  const snapshot = JSON.stringify({
    accent: { r: accent.r, g: accent.g, b: accent.b },
    ultraBlur: ultraBlurColors ?? null,
  });

  return (
    <div
      style={{
        position: "fixed",
        bottom: 12,
        right: 12,
        zIndex: 99999,
        background: "rgba(0,0,0,0.85)",
        borderRadius: 8,
        padding: 12,
        color: "#fff",
        fontSize: 11,
        fontFamily: "monospace",
        maxWidth: 340,
        pointerEvents: "auto",
      }}
    >
      <div style={{ marginBottom: 6, fontWeight: 700 }}>Color Debug</div>

      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
        <div
          style={{
            width: 20,
            height: 20,
            borderRadius: 4,
            background: `rgb(${accent.r},${accent.g},${accent.b})`,
            border: "1px solid rgba(255,255,255,0.2)",
            flexShrink: 0,
          }}
        />
        <span>accent: rgb({accent.r}, {accent.g}, {accent.b})</span>
      </div>

      {ultraBlurColors && (
        <div style={{ display: "flex", gap: 4, marginBottom: 6 }}>
          {(["topLeft", "topRight", "bottomLeft", "bottomRight"] as const).map((k) => (
            <div key={k} style={{ textAlign: "center" }}>
              <div
                style={{
                  width: 20,
                  height: 20,
                  borderRadius: 4,
                  background: `#${ultraBlurColors[k]}`,
                  border: "1px solid rgba(255,255,255,0.2)",
                }}
              />
              <div style={{ fontSize: 9, opacity: 0.6 }}>{k.replace("bottom", "b").replace("top", "t").replace("Left", "L").replace("Right", "R")}</div>
            </div>
          ))}
        </div>
      )}

      <textarea
        readOnly
        value={snapshot}
        onClick={(e) => (e.target as HTMLTextAreaElement).select()}
        style={{
          width: "100%",
          height: 60,
          background: "rgba(255,255,255,0.1)",
          color: "#ccc",
          border: "1px solid rgba(255,255,255,0.15)",
          borderRadius: 4,
          fontSize: 10,
          fontFamily: "monospace",
          resize: "none",
          padding: 4,
        }}
      />
    </div>
  );
}
