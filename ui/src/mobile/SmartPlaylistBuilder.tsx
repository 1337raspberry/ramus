import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { createSmartPlaylist, getSmartFilterChoices } from "../lib/commands";
import { pushBackHandler } from "../lib/backHandler";
import { useToastStore } from "../components/Toast";
import type { FilterChoice, Playlist, SmartFilter, SmartTerm } from "../lib/types";

interface Props {
  onCreated: (playlist: Playlist) => void;
  onDismiss: () => void;
}

// Select options compile to rules in buildFilter below; "" always means
// "any" (no rule emitted). Ratings use Plex's 10-scale; "Favourites" is the
// app's own fav semantic (userRating exactly 10).
const RATING_OPTIONS = [
  { value: "", label: "Any" },
  { value: "fav", label: "Favourites" },
  { value: "8", label: "Rated 8+" },
  { value: "6", label: "Rated 6+" },
  { value: "4", label: "Rated 4+" },
];

const PLAYS_OPTIONS = [
  { value: "", label: "Any" },
  { value: "never", label: "Never played" },
  { value: "played", label: "Played at least once" },
  { value: "10", label: "10+ plays" },
];

const LAST_PLAYED_OPTIONS = [
  { value: "", label: "Any" },
  { value: "in:7", label: "In the last week" },
  { value: "in:30", label: "In the last month" },
  { value: "in:90", label: "In the last 3 months" },
  { value: "not:30", label: "Not in the last month" },
  { value: "not:90", label: "Not in the last 3 months" },
  { value: "not:365", label: "Not in the last year" },
];

const ADDED_OPTIONS = [
  { value: "", label: "Any time" },
  { value: "7", label: "In the last week" },
  { value: "30", label: "In the last month" },
  { value: "90", label: "In the last 3 months" },
  { value: "365", label: "In the last year" },
];

const LIMIT_OPTIONS = [
  { value: "", label: "No limit" },
  { value: "25", label: "25 tracks" },
  { value: "50", label: "50 tracks" },
  { value: "100", label: "100 tracks" },
  { value: "250", label: "250 tracks" },
  { value: "500", label: "500 tracks" },
];

const SORT_OPTIONS = [
  { value: "", label: "Default order" },
  { value: "random", label: "Random" },
  { value: "userRating:desc", label: "Highest rated" },
  { value: "addedAt:desc", label: "Recently added" },
  { value: "lastViewedAt:desc", label: "Recently played" },
  { value: "viewCount:desc", label: "Most played" },
  { value: "mediaBitrate:desc", label: "Highest bitrate" },
  { value: "titleSort", label: "Title" },
];

/**
 * Full-form editor for creating a smart playlist: pick rules, a limit and a
 * sort, and the server stores the query — the playlist recomputes itself on
 * every open. Rules AND together; the genre multi-select ORs within
 * itself.
 */
export default function SmartPlaylistBuilder({ onCreated, onDismiss }: Props) {
  const [title, setTitle] = useState("");
  const [artist, setArtist] = useState("");
  const [genreChoices, setGenreChoices] = useState<FilterChoice[] | null>(null);
  const [genres, setGenres] = useState<Set<string>>(new Set());
  const [genresOpen, setGenresOpen] = useState(false);
  const [genreQuery, setGenreQuery] = useState("");
  const [rating, setRating] = useState("");
  const [yearFrom, setYearFrom] = useState("");
  const [yearTo, setYearTo] = useState("");
  const [plays, setPlays] = useState("");
  const [lastPlayed, setLastPlayed] = useState("");
  const [added, setAdded] = useState("");
  const [limit, setLimit] = useState("");
  const [sort, setSort] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    getSmartFilterChoices("album.genre")
      .then(setGenreChoices)
      .catch(() => setGenreChoices([]));
  }, []);

  useEffect(
    () =>
      pushBackHandler(() => {
        onDismiss();
        return true;
      }),
    [onDismiss],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDismiss]);

  const genreTitleById = useMemo(
    () => new Map((genreChoices ?? []).map((g) => [g.id, g.title])),
    [genreChoices],
  );

  const filteredGenres = useMemo(() => {
    if (!genreChoices) return [];
    const q = genreQuery.trim().toLowerCase();
    if (!q) return genreChoices;
    return genreChoices.filter((g) => g.title.toLowerCase().includes(q));
  }, [genreChoices, genreQuery]);

  const genreSummary = useMemo(() => {
    if (!genres.size) return "Any";
    const names = [...genres].map((id) => genreTitleById.get(id) ?? id);
    return names.length > 2
      ? `${names.slice(0, 2).join(", ")} +${names.length - 2}`
      : names.join(", ");
  }, [genres, genreTitleById]);

  const filter = useMemo<SmartFilter>(() => {
    const terms: SmartTerm[] = [];
    if (genres.size) terms.push({ field: "album.genre", op: "eq", values: [...genres] });
    const artistQuery = artist.trim();
    if (artistQuery) terms.push({ field: "artist.title", op: "eq", values: [artistQuery] });
    if (rating === "fav") {
      terms.push({ field: "track.userRating", op: "eq", values: ["10"] });
    } else if (rating) {
      // `gt` is strictly greater, so "N or higher" sends N-1.
      terms.push({ field: "track.userRating", op: "gt", values: [String(Number(rating) - 1)] });
    }
    // Same strictness dance for the inclusive year bounds. Years scope to
    // the album — tracks carry no year of their own.
    const from = parseInt(yearFrom, 10);
    if (!Number.isNaN(from))
      terms.push({ field: "album.year", op: "gt", values: [String(from - 1)] });
    const to = parseInt(yearTo, 10);
    if (!Number.isNaN(to)) terms.push({ field: "album.year", op: "lt", values: [String(to + 1)] });
    if (plays === "never") {
      terms.push({ field: "track.viewCount", op: "eq", values: ["0"] });
    } else if (plays === "played") {
      terms.push({ field: "track.viewCount", op: "gt", values: ["0"] });
    } else if (plays) {
      terms.push({ field: "track.viewCount", op: "gt", values: [String(Number(plays) - 1)] });
    }
    if (lastPlayed) {
      // "Not in the last N days" (date-before) also matches never-played
      // tracks — the right call for rediscovery lists; combine with a plays
      // rule to exclude them.
      const [kind, days] = lastPlayed.split(":");
      terms.push({
        field: "track.lastViewedAt",
        op: kind === "in" ? "gt" : "lt",
        values: [`-${days}d`],
      });
    }
    if (added) terms.push({ field: "track.addedAt", op: "gt", values: [`-${added}d`] });
    return { terms, sort: sort || null, limit: limit ? Number(limit) : null };
  }, [genres, artist, rating, yearFrom, yearTo, plays, lastPlayed, added, sort, limit]);

  const needsRuleOrLimit = filter.terms.length === 0 && filter.limit == null;
  const canCreate = !!title.trim() && !needsRuleOrLimit && !busy;

  const toggleGenre = (id: string) => {
    setGenres((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleCreate = () => {
    if (!canCreate) return;
    setBusy(true);
    createSmartPlaylist(title.trim(), filter)
      .then((p) => {
        useToastStore.getState().show(`Created “${p.title}”`);
        onCreated(p);
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't create the smart playlist");
        setBusy(false);
      });
  };

  // Portaled to <body>: the hub's ancestors carry transforms/filters that
  // turn `position: fixed` into "fixed within that ancestor", which squeezed
  // the panel between the toolbar and the mini player.
  return createPortal(
    <div
      className="settings-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div
        className="settings-panel glass smartpl-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="smartpl-heading"
      >
        <div className="settings-header">
          <h2 id="smartpl-heading">New Smart Playlist</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            x
          </button>
        </div>

        <div className="settings-body">
          <input
            className="smartpl-input smartpl-name"
            type="text"
            value={title}
            placeholder="Playlist name"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={(e) => setTitle(e.target.value)}
          />

          <div className="smartpl-rows">
            <button
              className="smartpl-row smartpl-expander"
              onClick={() => setGenresOpen((o) => !o)}
            >
              <span className="smartpl-row-label">Genres</span>
              <span className="smartpl-row-value">{genreSummary}</span>
            </button>
            {genresOpen && (
              <div className="smartpl-genre-pane">
                <input
                  className="smartpl-input"
                  type="text"
                  value={genreQuery}
                  placeholder="Filter genres"
                  autoComplete="off"
                  autoCorrect="off"
                  spellCheck={false}
                  onChange={(e) => setGenreQuery(e.target.value)}
                />
                <div className="smartpl-genre-list">
                  {genreChoices === null ? (
                    <div className="smartpl-genre-empty">Loading…</div>
                  ) : filteredGenres.length === 0 ? (
                    <div className="smartpl-genre-empty">
                      {genreChoices.length === 0 ? "Couldn't load genres" : "No matches"}
                    </div>
                  ) : (
                    filteredGenres.map((g) => (
                      <button
                        key={g.id}
                        className={`smartpl-genre-row${genres.has(g.id) ? " selected" : ""}`}
                        onClick={() => toggleGenre(g.id)}
                      >
                        {g.title}
                      </button>
                    ))
                  )}
                </div>
              </div>
            )}

            <div className="smartpl-row">
              <span className="smartpl-row-label">Artist contains</span>
              <input
                className="smartpl-input smartpl-grow"
                type="text"
                value={artist}
                placeholder="Any artist"
                autoComplete="off"
                autoCorrect="off"
                spellCheck={false}
                onChange={(e) => setArtist(e.target.value)}
              />
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Rating</span>
              <select
                className="sort-select"
                value={rating}
                onChange={(e) => setRating(e.target.value)}
              >
                {RATING_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Year</span>
              <input
                className="smartpl-input smartpl-year"
                type="text"
                inputMode="numeric"
                value={yearFrom}
                placeholder="From"
                onChange={(e) => setYearFrom(e.target.value.replace(/\D/g, ""))}
              />
              <span className="smartpl-year-sep">–</span>
              <input
                className="smartpl-input smartpl-year"
                type="text"
                inputMode="numeric"
                value={yearTo}
                placeholder="To"
                onChange={(e) => setYearTo(e.target.value.replace(/\D/g, ""))}
              />
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Plays</span>
              <select
                className="sort-select"
                value={plays}
                onChange={(e) => setPlays(e.target.value)}
              >
                {PLAYS_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Last played</span>
              <select
                className="sort-select"
                value={lastPlayed}
                onChange={(e) => setLastPlayed(e.target.value)}
              >
                {LAST_PLAYED_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Added</span>
              <select
                className="sort-select"
                value={added}
                onChange={(e) => setAdded(e.target.value)}
              >
                {ADDED_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Limit</span>
              <select
                className="sort-select"
                value={limit}
                onChange={(e) => setLimit(e.target.value)}
              >
                {LIMIT_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Sort</span>
              <select
                className="sort-select"
                value={sort}
                onChange={(e) => setSort(e.target.value)}
              >
                {SORT_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="settings-helper">
            {needsRuleOrLimit
              ? "Add at least one rule or a track limit."
              : "The playlist updates itself — tracks matching the rules join and leave automatically."}
          </div>

          <div className="smartpl-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="smartpl-btn" onClick={onDismiss}>
              Cancel
            </button>
            <button
              type="button"
              className="smartpl-btn smartpl-create"
              onClick={handleCreate}
              disabled={!canCreate}
            >
              {busy ? "Creating…" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
