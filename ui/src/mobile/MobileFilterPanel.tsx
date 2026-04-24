import { useCallback, useEffect, useState } from "react";
import {
  useLibraryStore,
  hasActiveFilters,
  countActiveFilters,
  DEFAULT_FILTERS,
  type AlbumFilters,
} from "../stores/libraryStore";
import { getDistinctCountries, getAllCollectionNames } from "../lib/commands";
import { countryToFlag } from "../lib/countryFlag";

interface Props {
  onDismiss: () => void;
}

export default function MobileFilterPanel({ onDismiss }: Props) {
  const filters = useLibraryStore((s) => s.albumFilters);
  const setFilters = useLibraryStore((s) => s.setAlbumFilters);
  const [countries, setCountries] = useState<string[]>([]);
  const [collections, setCollections] = useState<string[]>([]);

  useEffect(() => {
    getDistinctCountries()
      .then(setCountries)
      .catch(() => {});
    getAllCollectionNames()
      .then(setCollections)
      .catch(() => {});
  }, []);

  const update = useCallback(
    (patch: Partial<AlbumFilters>) => {
      setFilters({ ...filters, ...patch });
    },
    [filters, setFilters],
  );

  const activeCount = countActiveFilters(filters);

  return (
    <div className="mobile-filter-backdrop" onClick={onDismiss}>
      <div className="mobile-filter-panel" onClick={(e) => e.stopPropagation()}>
        <div className="mobile-filter-header">
          <span className="mobile-filter-title">
            Filters{activeCount > 0 && <span className="mobile-filter-count">{activeCount}</span>}
          </span>
          <button className="mobile-filter-done" onClick={onDismiss}>
            Done
          </button>
        </div>

        <div className="mobile-filter-body">
          <div className="mobile-filter-section">
            <label className="mobile-filter-toggle-row">
              <span>Unplayed</span>
              <input
                type="checkbox"
                checked={filters.unplayed}
                onChange={(e) => update({ unplayed: e.target.checked })}
              />
            </label>
            <label className="mobile-filter-toggle-row">
              <span>Favourite</span>
              <input
                type="checkbox"
                checked={filters.favourite}
                onChange={(e) => update({ favourite: e.target.checked })}
              />
            </label>
          </div>

          <div className="mobile-filter-section">
            <label className="mobile-filter-label">Year</label>
            <input
              className="mobile-filter-text"
              type="text"
              inputMode="numeric"
              value={filters.year}
              onChange={(e) => update({ year: e.target.value })}
              placeholder="1990, 1990-1999, >2000"
            />
            <div className="mobile-filter-decades">
              {["50s", "60s", "70s", "80s"].map((d) => {
                const base = 1900 + parseInt(d, 10);
                const range = `${base}-${base + 9}`;
                return (
                  <button
                    key={d}
                    className={`mobile-filter-decade${filters.year === range ? " active" : ""}`}
                    onClick={() => update({ year: filters.year === range ? "" : range })}
                  >
                    {d}
                  </button>
                );
              })}
            </div>
            <div className="mobile-filter-decades">
              {["90s", "00s", "10s", "20s"].map((d) => {
                const num = parseInt(d, 10);
                const base = num < 50 ? 2000 + num : 1900 + num;
                const range = `${base}-${base + 9}`;
                return (
                  <button
                    key={d}
                    className={`mobile-filter-decade${filters.year === range ? " active" : ""}`}
                    onClick={() => update({ year: filters.year === range ? "" : range })}
                  >
                    {d}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="mobile-filter-section">
            <label className="mobile-filter-label">Country</label>
            <select
              className="mobile-filter-select"
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

          <div className="mobile-filter-section">
            <label className="mobile-filter-label">Collection</label>
            <select
              className="mobile-filter-select"
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
        </div>

        {hasActiveFilters(filters) && (
          <button
            className="mobile-filter-clear"
            onClick={() => setFilters({ ...DEFAULT_FILTERS })}
          >
            Clear All Filters
          </button>
        )}
      </div>
    </div>
  );
}
