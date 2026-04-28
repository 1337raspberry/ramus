import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export interface ChipAutocompleteInputProps {
  /** Current chip values (the source of truth). Case-insensitive dedup happens
   * on commit. */
  value: string[];
  onChange: (next: string[]) => void;
  /** Resolve a query to a list of suggestions. May be sync (the parent has the
   * full list) or async (IPC-backed). Empty query is allowed and lets the
   * parent return e.g. the top-N alphabetical suggestions. */
  fetchSuggestions: (query: string) => Promise<string[]> | string[];
  placeholder?: string;
  /** Pre-render decoration for each chip (e.g. country flag). */
  renderChipPrefix?: (value: string) => React.ReactNode;
  /** Pre-render decoration for each suggestion row. */
  renderSuggestionPrefix?: (value: string) => React.ReactNode;
  /** When true, suggestions render inline below the input (mobile bottom-sheet
   * ergonomics — avoids floating popover that the iOS keyboard would obscure).
   * When false, the suggestion list is absolutely positioned. */
  inlineSuggestions?: boolean;
  /** Debounce in ms before firing fetchSuggestions. Default 100. */
  debounceMs?: number;
  /** Aria-label for the text input. */
  ariaLabel?: string;
}

/**
 * Chip-based multi-value autocomplete input.
 *
 * Keyboard:
 * - Enter: commit the highlighted suggestion, or commit the raw input if none.
 * - ArrowUp / ArrowDown: navigate suggestions.
 * - Backspace on empty: remove the last chip.
 * - Escape: close the suggestion list.
 *
 * Mouse:
 * - Click a suggestion to commit it.
 * - Click a chip's `x` to remove it.
 *
 * Duplicates are deduped case-insensitively on commit.
 */
export default function ChipAutocompleteInput({
  value,
  onChange,
  fetchSuggestions,
  placeholder,
  renderChipPrefix,
  renderSuggestionPrefix,
  inlineSuggestions = false,
  debounceMs = 100,
  ariaLabel,
}: ChipAutocompleteInputProps) {
  const [query, setQuery] = useState("");
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [highlightIndex, setHighlightIndex] = useState(0);
  const [open, setOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const fetchTokenRef = useRef(0);

  const valueLowerSet = useMemo(() => new Set(value.map((v) => v.toLowerCase())), [value]);

  const visibleSuggestions = useMemo(
    () => suggestions.filter((s) => !valueLowerSet.has(s.toLowerCase())),
    [suggestions, valueLowerSet],
  );

  // Debounced suggestion fetch. Token-guarded so a slow IPC reply from a
  // previous query can't overwrite the current one.
  useEffect(() => {
    if (!open) return;
    const token = ++fetchTokenRef.current;
    const timer = window.setTimeout(async () => {
      try {
        const result = await Promise.resolve(fetchSuggestions(query));
        if (fetchTokenRef.current !== token) return;
        setSuggestions(result);
        setHighlightIndex(0);
      } catch {
        if (fetchTokenRef.current !== token) return;
        setSuggestions([]);
      }
    }, debounceMs);
    return () => window.clearTimeout(timer);
  }, [query, open, fetchSuggestions, debounceMs]);

  // Clamp `highlightIndex` when the visible-suggestions list shrinks (e.g.
  // user picks a chip → that suggestion drops out of the list). Without
  // this, Enter could miss the array end and fall through to commit the
  // raw query.
  useEffect(() => {
    if (highlightIndex >= visibleSuggestions.length && visibleSuggestions.length > 0) {
      setHighlightIndex(visibleSuggestions.length - 1);
    }
  }, [visibleSuggestions, highlightIndex]);

  // Outside-click close (skipped in inline mode — the parent panel handles it).
  useEffect(() => {
    if (!open || inlineSuggestions) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open, inlineSuggestions]);

  const commit = useCallback(
    (raw: string) => {
      const trimmed = raw.trim();
      if (!trimmed) return;
      if (valueLowerSet.has(trimmed.toLowerCase())) {
        setQuery("");
        return;
      }
      onChange([...value, trimmed]);
      setQuery("");
      setHighlightIndex(0);
    },
    [onChange, value, valueLowerSet],
  );

  const remove = useCallback(
    (index: number) => {
      const next = value.slice();
      next.splice(index, 1);
      onChange(next);
    },
    [onChange, value],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (visibleSuggestions.length > 0 && highlightIndex < visibleSuggestions.length) {
          commit(visibleSuggestions[highlightIndex]);
        } else if (query.trim()) {
          commit(query);
        }
      } else if (e.key === "Backspace" && query === "" && value.length > 0) {
        e.preventDefault();
        remove(value.length - 1);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setOpen(true);
        setHighlightIndex((i) => Math.min(visibleSuggestions.length - 1, i + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setHighlightIndex((i) => Math.max(0, i - 1));
      } else if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
      }
    },
    [commit, highlightIndex, query, remove, value.length, visibleSuggestions],
  );

  const showList = open && (visibleSuggestions.length > 0 || query.trim().length > 0);

  return (
    <div className={`chip-autocomplete${inlineSuggestions ? " inline" : ""}`} ref={wrapRef}>
      <div
        className="chip-autocomplete-row"
        onClick={() => {
          inputRef.current?.focus();
          setOpen(true);
        }}
      >
        {value.map((v, i) => (
          <span key={`${v}-${i}`} className="chip">
            {renderChipPrefix && <span className="chip-prefix">{renderChipPrefix(v)}</span>}
            <span className="chip-label">{v}</span>
            <button
              type="button"
              className="chip-remove"
              aria-label={`Remove ${v}`}
              onClick={(e) => {
                e.stopPropagation();
                remove(i);
              }}
            >
              ×
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          className="chip-autocomplete-input"
          type="text"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={handleKeyDown}
          placeholder={value.length === 0 ? placeholder : ""}
          aria-label={ariaLabel}
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
        />
      </div>
      {showList && (
        <div
          className={`chip-autocomplete-suggestions${inlineSuggestions ? " inline" : ""}`}
          role="listbox"
        >
          {visibleSuggestions.length === 0 ? (
            <div className="chip-suggestion empty">No matches — Enter to add</div>
          ) : (
            visibleSuggestions.map((s, i) => (
              <button
                key={s}
                type="button"
                role="option"
                aria-selected={i === highlightIndex}
                className={`chip-suggestion${i === highlightIndex ? " highlighted" : ""}`}
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setHighlightIndex(i)}
                onClick={() => commit(s)}
              >
                {renderSuggestionPrefix && (
                  <span className="chip-suggestion-prefix">{renderSuggestionPrefix(s)}</span>
                )}
                <span className="chip-suggestion-label">{s}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
