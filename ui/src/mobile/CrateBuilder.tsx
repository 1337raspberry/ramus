import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  createCratePlaylist,
  estimateCrate,
  getGenreSuggestions,
  updateCratePlaylist,
} from "../lib/commands";
import { pushBackHandler } from "../lib/backHandler";
import { useToastStore } from "../components/Toast";
import KeyboardDoneBar from "./KeyboardDoneBar";
import type { CrateEstimate, CrateRecipe, CrateUpdate, Playlist } from "../lib/types";

interface Props {
  /** Present when editing an existing crate; absent when creating one. */
  existing?: { sourceId: string; recipe: CrateRecipe };
  onCreated?: (playlist: Playlist) => void;
  onSaved?: (update: CrateUpdate, recipe: CrateRecipe) => void;
  onDismiss: () => void;
}

const HOT_OPTIONS = [
  { value: "1", label: "Best track per album" },
  { value: "2", label: "Best 2 per album" },
  { value: "3", label: "Best 3 per album" },
];

const COUNT_OPTIONS = [
  { value: "10", label: "10 tracks" },
  { value: "25", label: "25 tracks" },
  { value: "50", label: "50 tracks" },
  { value: "100", label: "100 tracks" },
];

const GENRE_SUGGESTION_LIMIT = 40;

/** Snap a stored value onto the options the control offers, so a legacy
 * recipe (uncapped, or with the per-album rule off) still loads for editing
 * rather than rendering a select with no matching option. */
const clampOption = (value: number, options: { value: string }[], fallback: string) =>
  options.some((o) => o.value === String(value)) ? String(value) : fallback;

/**
 * Editor for a discovery crate. Unlike a smart playlist, the rules run here
 * rather than on the server — popularity ranking and genre-tree expansion
 * have no equivalent in Plex's filter grammar — so the result is a fixed
 * track list that a Regenerate action re-rolls on demand.
 *
 * The playlist's name is derived from the rules (in Rust, alongside the
 * recipe parser), never typed: editing a crate renames it to match, so the
 * title always states what's inside.
 */
export default function CrateBuilder({ existing, onCreated, onSaved, onDismiss }: Props) {
  const [genre, setGenre] = useState<string | null>(existing?.recipe.genres[0] ?? null);
  const [genreQuery, setGenreQuery] = useState("");
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [includeSubgenres, setIncludeSubgenres] = useState(
    existing?.recipe.includeSubgenres ?? true,
  );
  const [hotPerAlbum, setHotPerAlbum] = useState(
    existing ? clampOption(existing.recipe.hotPerAlbum, HOT_OPTIONS, "2") : "2",
  );
  const [unplayedOnly, setUnplayedOnly] = useState(existing?.recipe.unplayedOnly ?? true);
  const [count, setCount] = useState(
    existing ? clampOption(existing.recipe.count, COUNT_OPTIONS, "25") : "25",
  );
  const [estimate, setEstimate] = useState<CrateEstimate | null>(null);
  const [estimating, setEstimating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [fieldFocused, setFieldFocused] = useState(false);

  // Moving between fields fires blur then focus, so reacting to blur
  // directly would flash the Done bar out and back. Defer it by a tick and
  // let an arriving focus cancel it.
  const blurTimer = useRef<number | null>(null);
  const onFieldFocus = () => {
    if (blurTimer.current !== null) {
      clearTimeout(blurTimer.current);
      blurTimer.current = null;
    }
    setFieldFocused(true);
  };
  const onFieldBlur = () => {
    blurTimer.current = window.setTimeout(() => setFieldFocused(false), 0);
  };
  useEffect(
    () => () => {
      if (blurTimer.current !== null) clearTimeout(blurTimer.current);
    },
    [],
  );

  const recipe = useMemo<CrateRecipe>(
    () => ({
      genres: genre ? [genre] : [],
      includeSubgenres,
      hotPerAlbum: Number(hotPerAlbum),
      unplayedOnly,
      count: Number(count),
    }),
    [genre, includeSubgenres, hotPerAlbum, unplayedOnly, count],
  );

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

  // Results appear only once something is typed. An empty query returns the
  // first N tags alphabetically, which is too few of ~600 to browse and only
  // ever showed the A's — so it read as the whole genre list while costing a
  // large chunk of the form's height.
  const searching = genre === null && genreQuery.trim().length > 0;

  useEffect(() => {
    const query = genreQuery.trim();
    if (!query) {
      setSuggestions([]);
      return;
    }
    let cancelled = false;
    const id = setTimeout(() => {
      getGenreSuggestions(query, GENRE_SUGGESTION_LIMIT)
        .then((names) => {
          if (!cancelled) setSuggestions(names);
        })
        .catch(() => {
          if (!cancelled) setSuggestions([]);
        });
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(id);
    };
  }, [genreQuery]);

  // The estimate is a whole-library query, so it trails the controls rather
  // than firing per keystroke. A stale reply must never land on top of a
  // newer one — the counts drive the action button's copy.
  const estimateSeq = useRef(0);
  useEffect(() => {
    if (recipe.genres.length === 0) {
      setEstimate(null);
      setEstimating(false);
      return;
    }
    const seq = ++estimateSeq.current;
    setEstimating(true);
    const id = setTimeout(() => {
      estimateCrate(recipe)
        .then((result) => {
          if (seq !== estimateSeq.current) return;
          setEstimate(result);
          setEstimating(false);
        })
        .catch(() => {
          if (seq !== estimateSeq.current) return;
          setEstimate(null);
          setEstimating(false);
        });
    }, 250);
    return () => clearTimeout(id);
  }, [recipe]);

  const pickGenre = (name: string) => {
    setGenre(name);
    setGenreQuery("");
    setSuggestions([]);
  };

  const clearGenre = () => {
    setGenre(null);
    setEstimate(null);
  };

  const canSubmit = genre !== null && !busy && (estimate?.selected ?? 0) > 0;

  const handleCreate = () => {
    if (!canSubmit) return;
    setBusy(true);
    createCratePlaylist(recipe)
      .then((p) => {
        useToastStore.getState().show(`Created “${p.title}”`);
        onCreated?.(p);
      })
      .catch((e) => {
        useToastStore.getState().show(String(e) || "Couldn't create the crate");
        setBusy(false);
      });
  };

  const handleSave = () => {
    if (!canSubmit || !existing) return;
    setBusy(true);
    updateCratePlaylist(existing.sourceId, recipe)
      .then((update) => {
        useToastStore.getState().show(`Updated — ${update.items.length} tracks`);
        onSaved?.(update, recipe);
      })
      .catch((e) => {
        useToastStore.getState().show(String(e) || "Couldn't update the crate");
        setBusy(false);
      });
  };

  const heading = existing ? "Edit Crate" : "New Discovery Crate";

  // Portaled to <body>: the hub's ancestors carry transforms/filters that
  // turn `position: fixed` into "fixed within that ancestor".
  const panel = createPortal(
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
        aria-labelledby="crate-heading"
      >
        <div className="settings-header">
          <h2 id="crate-heading">{heading}</h2>
          <button className="settings-close" onClick={onDismiss} aria-label="Close">
            x
          </button>
        </div>

        <div className="settings-body">
          <div className={`crate-title-preview${estimating ? " pending" : ""}`}>
            {genre === null
              ? "Pick a genre to name your crate"
              : (estimate?.derivedTitle ?? "…")}
          </div>

          <div className="smartpl-rows">
            <div className="smartpl-genre-pane">
              {genre === null ? (
                <>
                  <input
                    className="smartpl-input"
                    type="text"
                    value={genreQuery}
                    placeholder="Search genres"
                    autoComplete="off"
                    autoCorrect="off"
                    spellCheck={false}
                    onFocus={onFieldFocus}
                    onBlur={onFieldBlur}
                    onChange={(e) => setGenreQuery(e.target.value)}
                  />
                  {searching && (
                    <div className="smartpl-genre-list">
                      {suggestions.length === 0 ? (
                        <div className="smartpl-genre-empty">No matches</div>
                      ) : (
                        suggestions.map((name) => (
                          <button
                            key={name}
                            className="smartpl-genre-row"
                            onClick={() => pickGenre(name)}
                          >
                            {name}
                          </button>
                        ))
                      )}
                    </div>
                  )}
                </>
              ) : (
                <div className="crate-genre-selected">
                  <span className="crate-genre-name">{genre}</span>
                  <button
                    className="crate-genre-clear"
                    onClick={clearGenre}
                    aria-label="Change genre"
                  >
                    ×
                  </button>
                </div>
              )}
            </div>

            <label className="smartpl-row">
              <span className="smartpl-row-label">Include sub-genres</span>
              <input
                type="checkbox"
                checked={includeSubgenres}
                onChange={(e) => setIncludeSubgenres(e.target.checked)}
              />
            </label>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Tracks per album</span>
              <select
                className="sort-select"
                value={hotPerAlbum}
                onChange={(e) => setHotPerAlbum(e.target.value)}
              >
                {HOT_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <label className="smartpl-row">
              <span className="smartpl-row-label">Unplayed only</span>
              <input
                type="checkbox"
                checked={unplayedOnly}
                onChange={(e) => setUnplayedOnly(e.target.checked)}
              />
            </label>

            <div className="smartpl-row">
              <span className="smartpl-row-label">Crate Size</span>
              <select
                className="sort-select"
                value={count}
                onChange={(e) => setCount(e.target.value)}
              >
                {COUNT_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="settings-helper">
            {genre === null
              ? "Pick a genre."
              : estimating
                ? "Working out what matches…"
                : estimate
                  ? `${estimate.genreTags} genre ${
                      estimate.genreTags === 1 ? "tag" : "tags"
                    } · ${estimate.candidateAlbums} albums · ${
                      estimate.eligibleTracks
                    } of ${estimate.candidateTracks} tracks match · ${estimate.selected} selected`
                  : "Couldn't work out what matches."}
            <br />
            <br />
            Each crate is generated at creation time. Use the option menu in each playlist to
            recalculate and regenerate a new set of tracks.
          </div>

          <div className="smartpl-actions">
            <div style={{ flex: 1 }} />
            <button type="button" className="smartpl-btn" onClick={onDismiss}>
              Cancel
            </button>
            <button
              type="button"
              className="smartpl-btn smartpl-create"
              onClick={existing ? handleSave : handleCreate}
              disabled={!canSubmit}
            >
              {existing ? (busy ? "Saving…" : "Save") : busy ? "Creating…" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );

  return (
    <>
      {panel}
      <KeyboardDoneBar visible={fieldFocused} />
    </>
  );
}
