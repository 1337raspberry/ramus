import type { DescriptionSegment, GenreMetadata } from "../lib/types";

// The importer caps the overall payload (64 MB) but not per-field length, so
// a degenerate tree can carry a multi-MB description. Bound what we mount —
// one DOM node per segment over unbounded text would hang the render.
const MAX_DESCRIPTION_CHARS = 4000;

function clampSegments(segments: DescriptionSegment[]): DescriptionSegment[] {
  let budget = MAX_DESCRIPTION_CHARS;
  const out: DescriptionSegment[] = [];
  for (const seg of segments) {
    if (budget <= 0) {
      out.push({ kind: "text", value: "…" });
      break;
    }
    if (seg.kind === "text" && seg.value.length > budget) {
      out.push({ kind: "text", value: seg.value.slice(0, budget) + "…" });
      budget = 0;
      continue;
    }
    // Links are short names — never sliced mid-word.
    out.push(seg);
    budget -= seg.value.length;
  }
  return out;
}

interface Props {
  meta: GenreMetadata | null;
  loading: boolean;
  /** Genre references always drill in-place rather than navigating away. */
  onDrillGenre: (genre: string) => void;
  /** Called only for artists that exist in the library. */
  onNavigateArtist: (artist: string) => void;
}

/**
 * Body of the genre-info surfaces: short summary, AKA list, and the
 * marked-up description segments. Platform-neutral — the mobile sheet and
 * desktop modal wrap it in their own chrome and scroll containers.
 */
export default function GenreInfoContent({ meta, loading, onDrillGenre, onNavigateArtist }: Props) {
  const shortSummary = meta?.shortSummary ?? null;
  const akas = meta?.cosmeticAka ?? [];
  const segments = clampSegments(meta?.descriptionSegments ?? []);
  const showMinimal = !loading && !shortSummary && akas.length === 0 && segments.length === 0;

  return (
    <>
      {shortSummary && <p className="genre-info-short">{shortSummary}</p>}

      {akas.length > 0 && (
        <div className="genre-info-section">
          <div className="genre-info-label">AKA</div>
          <div className="genre-info-aka">
            {/* Index key: imported cosmetic_aka arrays aren't deduplicated. */}
            {akas.map((aka, i) => (
              <span key={i}>
                {i > 0 && <span className="genre-info-aka-sep"> · </span>}
                {aka}
              </span>
            ))}
          </div>
        </div>
      )}

      {segments.length > 0 && (
        <div className="genre-info-section">
          <p className="genre-info-summary">
            {segments.map((seg, i) => {
              if (seg.kind === "text") return <span key={i}>{seg.value}</span>;
              if (seg.kind === "genreLink") {
                // Genre links always drill into the genre's info; the
                // library flag only adds the bold + underlined treatment.
                return (
                  <button
                    key={i}
                    className={`genre-link${seg.inLibrary ? " owned" : ""}`}
                    onClick={() => onDrillGenre(seg.value)}
                  >
                    {seg.value}
                  </button>
                );
              }
              // Artist links navigate only when owned; otherwise they're
              // accent-coloured but non-interactive.
              if (seg.inLibrary) {
                const navName = seg.navName ?? seg.value;
                return (
                  <button
                    key={i}
                    className="artist-link owned"
                    onClick={() => onNavigateArtist(navName)}
                  >
                    {seg.value}
                  </button>
                );
              }
              return (
                <span key={i} className="artist-ref">
                  {seg.value}
                </span>
              );
            })}
          </p>
        </div>
      )}

      {showMinimal && <div className="genre-info-empty">No additional info</div>}
    </>
  );
}
