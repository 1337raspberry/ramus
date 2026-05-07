import { useCallback, useId, useRef } from "react";

interface TabDef<T extends string> {
  id: T;
  label: string;
}

interface TabBarProps<T extends string> {
  tabs: ReadonlyArray<TabDef<T>>;
  active: T;
  onChange: (next: T) => void;
  /// Optional id passed through as `aria-controls` on each tab so screen
  /// readers know which tabpanel a tab governs. Pair with `tabPanelId`
  /// on the rendered panel container.
  panelId?: string;
  /// Visible label for the tablist itself, surfaced as `aria-label` —
  /// helps SR users distinguish multiple tablists on the same page.
  ariaLabel?: string;
}

/// Generic tab strip — accent-color underline on the active tab, uppercase
/// label, 36px tall (44px on coarse pointers via styles.css). Shares the
/// visual language of the sidebar-tabs but isn't coupled to the sidebar
/// — designed to drop in anywhere a horizontal section switcher is needed.
///
/// Implements the WAI-ARIA tablist keyboard model: ArrowLeft/ArrowRight
/// move the active tab, Home/End jump to first/last. Only the currently
/// active tab is in the focus order (`tabIndex={0}` on active, `-1` on
/// the rest) so Tab steps over the strip in one stop.
export function TabBar<T extends string>({
  tabs,
  active,
  onChange,
  panelId,
  ariaLabel,
}: TabBarProps<T>) {
  const tablistId = useId();
  const buttonRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const handleKey = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>, idx: number) => {
      let nextIdx: number | null = null;
      switch (e.key) {
        case "ArrowRight":
          nextIdx = (idx + 1) % tabs.length;
          break;
        case "ArrowLeft":
          nextIdx = (idx - 1 + tabs.length) % tabs.length;
          break;
        case "Home":
          nextIdx = 0;
          break;
        case "End":
          nextIdx = tabs.length - 1;
          break;
        default:
          return;
      }
      if (nextIdx == null) return;
      e.preventDefault();
      onChange(tabs[nextIdx].id);
      buttonRefs.current[nextIdx]?.focus();
    },
    [onChange, tabs],
  );

  return (
    <div className="tabs" role="tablist" aria-label={ariaLabel}>
      {tabs.map((t, idx) => {
        const isActive = active === t.id;
        const tabId = `${tablistId}-tab-${t.id}`;
        return (
          <button
            key={t.id}
            ref={(el) => {
              buttonRefs.current[idx] = el;
            }}
            id={tabId}
            type="button"
            role="tab"
            aria-selected={isActive}
            aria-controls={panelId}
            tabIndex={isActive ? 0 : -1}
            className={`tab${isActive ? " active" : ""}`}
            onClick={() => onChange(t.id)}
            onKeyDown={(e) => handleKey(e, idx)}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}
