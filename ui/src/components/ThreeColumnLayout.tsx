import { useCallback, useEffect, useRef, useState } from "react";

const STORAGE_KEY = "ramus-column-widths";

const SIDEBAR_MIN = 180;
const SIDEBAR_MAX = 350;
const SIDEBAR_DEFAULT = 220;

const DETAIL_MIN = 280;
const DETAIL_MAX = 800;
const DETAIL_DEFAULT = 420;

interface Props {
  sidebar: React.ReactNode;
  content: React.ReactNode;
  detail: React.ReactNode;
}

function loadWidths(): { sidebar: number; detail: number } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        sidebar: Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, parsed.sidebar ?? SIDEBAR_DEFAULT)),
        detail: Math.max(DETAIL_MIN, Math.min(DETAIL_MAX, parsed.detail ?? DETAIL_DEFAULT)),
      };
    }
  } catch {
    // ignore
  }
  return { sidebar: SIDEBAR_DEFAULT, detail: DETAIL_DEFAULT };
}

function saveWidths(sidebar: number, detail: number) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ sidebar, detail }));
}

export default function ThreeColumnLayout({ sidebar, content, detail }: Props) {
  const [widths, setWidths] = useState(loadWidths);
  const dragging = useRef<"left" | "right" | null>(null);
  const startX = useRef(0);
  const startWidth = useRef(0);

  const onMouseDown = useCallback(
    (side: "left" | "right", e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = side;
      startX.current = e.clientX;
      startWidth.current =
        side === "left" ? widths.sidebar : widths.detail;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [widths]
  );

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return;
      const delta = e.clientX - startX.current;

      if (dragging.current === "left") {
        const next = Math.max(
          SIDEBAR_MIN,
          Math.min(SIDEBAR_MAX, startWidth.current + delta)
        );
        setWidths((prev) => ({ ...prev, sidebar: next }));
      } else {
        // Right divider: dragging right = narrower detail
        const next = Math.max(
          DETAIL_MIN,
          Math.min(DETAIL_MAX, startWidth.current - delta)
        );
        setWidths((prev) => ({ ...prev, detail: next }));
      }
    };

    const onMouseUp = () => {
      if (dragging.current) {
        dragging.current = null;
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        setWidths((w) => {
          saveWidths(w.sidebar, w.detail);
          return w;
        });
      }
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  return (
    <div
      className="three-column-layout"
      style={{
        gridTemplateColumns: `${widths.sidebar}px 5px 1fr 5px ${widths.detail}px`,
      }}
    >
      <div className="column column-sidebar">{sidebar}</div>
      <div
        className={`column-divider${dragging.current === "left" ? " dragging" : ""}`}
        onMouseDown={(e) => onMouseDown("left", e)}
      />
      <div className="column column-content">{content}</div>
      <div
        className={`column-divider${dragging.current === "right" ? " dragging" : ""}`}
        onMouseDown={(e) => onMouseDown("right", e)}
      />
      <div className="column column-detail">{detail}</div>
    </div>
  );
}
