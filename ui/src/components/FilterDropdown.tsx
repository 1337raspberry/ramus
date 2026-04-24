import { useCallback, useEffect, useRef, useState } from "react";
import {
  useLibraryStore,
  hasActiveFilters,
  countActiveFilters,
  DEFAULT_FILTERS,
  type AlbumFilters,
} from "../stores/libraryStore";
import { getDistinctCountries, getAllCollectionNames } from "../lib/commands";
import { IconFilter } from "./Icons";
import { countryToFlag } from "../lib/countryFlag";

export default function FilterDropdown() {
  const filters = useLibraryStore((s) => s.albumFilters);
  const setFilters = useLibraryStore((s) => s.setAlbumFilters);
  const [open, setOpen] = useState(false);
  const [countries, setCountries] = useState<string[]>([]);
  const [collections, setCollections] = useState<string[]>([]);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    getDistinctCountries()
      .then(setCountries)
      .catch(() => {});
    getAllCollectionNames()
      .then(setCollections)
      .catch(() => {});
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const update = useCallback(
    (patch: Partial<AlbumFilters>) => {
      setFilters({ ...filters, ...patch });
    },
    [filters, setFilters],
  );

  const activeCount = countActiveFilters(filters);
  const canClear = hasActiveFilters(filters);

  return (
    <div className="filter-dropdown-wrap" ref={wrapRef}>
      <button
        className={`filter-dropdown-btn${activeCount > 0 ? " active" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title="Filter albums"
      >
        <IconFilter size={14} />
        {activeCount > 0 && <span className="filter-badge">{activeCount}</span>}
      </button>
      {open && (
        <div className="filter-dropdown-panel">
          <div className="filter-section">
            <label className="filter-check-row">
              <input
                type="checkbox"
                checked={filters.unplayed}
                onChange={(e) => update({ unplayed: e.target.checked })}
              />
              <span>Unplayed</span>
            </label>
            <label className="filter-check-row">
              <input
                type="checkbox"
                checked={filters.favourite}
                onChange={(e) => update({ favourite: e.target.checked })}
              />
              <span>Favourite</span>
            </label>
          </div>

          <div className="filter-section">
            <label className="filter-field-label">Year</label>
            <input
              className="filter-text-input"
              type="text"
              value={filters.year}
              onChange={(e) => update({ year: e.target.value })}
              placeholder="1990, 1990-1999, >2000"
            />
            <div className="filter-decades">
              {["50s", "60s", "70s", "80s", "90s", "00s", "10s", "20s"].map((d) => {
                const num = parseInt(d, 10);
                const base = num < 50 ? 2000 + num : 1900 + num;
                const range = `${base}-${base + 9}`;
                return (
                  <button
                    key={d}
                    className={`filter-decade${filters.year === range ? " active" : ""}`}
                    onClick={() => update({ year: filters.year === range ? "" : range })}
                  >
                    {d}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="filter-section">
            <label className="filter-field-label">Country</label>
            <select
              className="filter-select"
              value={filters.country}
              onChange={(e) => update({ country: e.target.value })}
            >
              <option value="">All</option>
              {countries.map((c) => {
                const flag = countryToFlag(c);
                return (
                  <option key={c} value={c}>
                    {flag ? `${flag} ${c}` : c}
                  </option>
                );
              })}
            </select>
          </div>

          <div className="filter-section">
            <label className="filter-field-label">Collection</label>
            <select
              className="filter-select"
              value={filters.collection}
              onChange={(e) => update({ collection: e.target.value })}
            >
              <option value="">All</option>
              {collections.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          </div>

          {canClear && (
            <button className="filter-clear-btn" onClick={() => setFilters({ ...DEFAULT_FILTERS })}>
              Clear all
            </button>
          )}
        </div>
      )}
    </div>
  );
}
