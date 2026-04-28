import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  useLibraryStore,
  hasActiveFilters,
  countActiveFilters,
  DEFAULT_FILTERS,
  type AlbumFilters,
} from "../stores/libraryStore";
import { getDistinctCountries, getAllCollectionNames, getGenreSuggestions } from "../lib/commands";
import { usePlaybackStore } from "../stores/playbackStore";
import { countryToFlag } from "../lib/countryFlag";
import { filterCountrySuggestions } from "../lib/filterSuggestions";
import ChipAutocompleteInput from "../components/ChipAutocompleteInput";
import FilterPanelMenu from "../components/FilterPanelMenu";

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

  // Stable refs so ChipAutocompleteInput's debounce effect isn't reset on
  // every parent render. See the equivalent block in FilterDropdown.
  const fetchGenres = useCallback((q: string) => getGenreSuggestions(q), []);
  const fetchCountries = useCallback(
    (q: string) => filterCountrySuggestions(countries, q, 200),
    [countries],
  );

  const activeCount = countActiveFilters(filters);

  const hasTrack = !!usePlaybackStore((s) => s.currentTrack);

  return createPortal(
    <div className="mobile-filter-backdrop" onClick={onDismiss}>
      <div
        className={`mobile-filter-panel${hasTrack ? " with-mini" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mobile-filter-header">
          <div className="mobile-filter-title-group">
            <span className="mobile-filter-title">
              Filters
              {activeCount > 0 && <span className="mobile-filter-count">{activeCount}</span>}
            </span>
            <FilterPanelMenu onAfterAction={onDismiss} />
          </div>
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
              <span>Favourite Albums</span>
              <input
                type="checkbox"
                checked={filters.favouriteAlbums}
                onChange={(e) => update({ favouriteAlbums: e.target.checked })}
              />
            </label>
            <label className="mobile-filter-toggle-row">
              <span>Favourite Tracks</span>
              <input
                type="checkbox"
                checked={filters.favouriteTracks}
                onChange={(e) => update({ favouriteTracks: e.target.checked })}
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
            <label className="mobile-filter-label">Genres</label>
            <ChipAutocompleteInput
              value={filters.genres}
              onChange={(genres) => update({ genres })}
              fetchSuggestions={fetchGenres}
              placeholder="Type a genre…"
              ariaLabel="Genre filter"
              inlineSuggestions
            />
          </div>

          <div className="mobile-filter-section">
            <label className="mobile-filter-label">Countries</label>
            <ChipAutocompleteInput
              value={filters.countries}
              onChange={(countries) => update({ countries })}
              fetchSuggestions={fetchCountries}
              renderChipPrefix={(c) => countryToFlag(c) ?? null}
              renderSuggestionPrefix={(c) => countryToFlag(c) ?? null}
              placeholder="Type a country…"
              ariaLabel="Country filter"
              inlineSuggestions
            />
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
    </div>,
    document.body,
  );
}
