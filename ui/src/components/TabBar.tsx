interface TabDef<T extends string> {
  id: T;
  label: string;
}

interface TabBarProps<T extends string> {
  tabs: ReadonlyArray<TabDef<T>>;
  active: T;
  onChange: (next: T) => void;
}

/// Generic tab strip — accent-color underline on the active tab, uppercase
/// label, 36px tall (44px on coarse pointers via styles.css). Shares the
/// visual language of the sidebar-tabs but isn't coupled to the sidebar
/// — designed to drop in anywhere a horizontal section switcher is needed.
export function TabBar<T extends string>({ tabs, active, onChange }: TabBarProps<T>) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          role="tab"
          aria-selected={active === t.id}
          className={`tab${active === t.id ? " active" : ""}`}
          onClick={() => onChange(t.id)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}
