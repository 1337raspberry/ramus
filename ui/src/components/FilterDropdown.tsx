import { useCallback, useEffect, useRef, useState } from "react";
import {
  useLibraryStore,
  hasActiveFilters,
  countActiveFilters,
  DEFAULT_FILTERS,
  type AlbumFilters,
} from "../stores/libraryStore";
import { getDistinctCountries, getAllCollectionNames, getGenreSuggestions } from "../lib/commands";
import { IconFilter } from "./Icons";
import { countryToFlag } from "../lib/countryFlag";
import { filterCountrySuggestions } from "../lib/filterSuggestions";
import ChipAutocompleteInput from "./ChipAutocompleteInput";
import FilterPanelMenu from "./FilterPanelMenu";

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
      const target = e.target as Element | null;
      if (!target || !wrapRef.current) return;
      if (wrapRef.current.contains(target)) return;
      // Don't dismiss while a modal triggered from inside the dropdown
      // (e.g. the bookmark save dialog) is open: the modal lives in this
      // dropdown's React tree, so closing the dropdown unmounts it before
      // the modal's click handler fires. The modal owns its own dismiss.
      if (target.closest(".settings-backdrop")) return;
      setOpen(false);
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

  // Stable references prevent ChipAutocompleteInput's debounce effect from
  // re-firing on every parent render. Genre suggestions are IPC-backed and
  // don't depend on local state, so the closure has no deps. Country
  // suggestions close over `countries` so they bust when the cached country
  // list arrives.
  const fetchGenres = useCallback((q: string) => getGenreSuggestions(q), []);
  const fetchCountries = useCallback(
    (q: string) => filterCountrySuggestions(countries, q, 200),
    [countries],
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
          <div className="filter-panel-header">
            <FilterPanelMenu onAfterAction={() => setOpen(false)} />
          </div>
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
                checked={filters.favouriteAlbums}
                onChange={(e) => update({ favouriteAlbums: e.target.checked })}
              />
              <span>Favourite Albums</span>
            </label>
            <label className="filter-check-row">
              <input
                type="checkbox"
                checked={filters.favouriteTracks}
                onChange={(e) => update({ favouriteTracks: e.target.checked })}
              />
              <span>Favourite Tracks</span>
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
            <label className="filter-field-label">Genres</label>
            <ChipAutocompleteInput
              value={filters.genres}
              onChange={(genres) => update({ genres })}
              fetchSuggestions={fetchGenres}
              placeholder="Type a genre…"
              ariaLabel="Genre filter"
            />
          </div>

          <div className="filter-section">
            <label className="filter-field-label">Countries</label>
            <ChipAutocompleteInput
              value={filters.countries}
              onChange={(countries) => update({ countries })}
              fetchSuggestions={fetchCountries}
              renderChipPrefix={(c) => countryToFlag(c) ?? null}
              renderSuggestionPrefix={(c) => countryToFlag(c) ?? null}
              placeholder="Type a country…"
              ariaLabel="Country filter"
            />
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
