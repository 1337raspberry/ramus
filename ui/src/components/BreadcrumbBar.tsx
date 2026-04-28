import { useCallback, useMemo } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useBreadcrumbDebugStore } from "./BreadcrumbDebugPanel";
import { countryToFlag } from "../lib/countryFlag";
import type { GenreNode } from "../lib/types";

interface Crumb {
  label: string;
  flag?: string | null;
  onClick?: () => void;
}

/** Walk the genre tree to find a node by its path-based id. */
function findNodeById(nodes: GenreNode[], id: string): GenreNode | null {
  for (const n of nodes) {
    if (n.id === id) return n;
    if (n.children) {
      const found = findNodeById(n.children, id);
      if (found) return found;
    }
  }
  return null;
}

export default function BreadcrumbBar() {
  const genreTree = useLibraryStore((s) => s.genreTree);
  const selectedGenreId = useLibraryStore((s) => s.selectedGenreId);
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const searchQuery = useLibraryStore((s) => s.searchQuery);
  const activeBookmarkName = useLibraryStore((s) => s.activeBookmarkName);
  const browseArtistName = useLibraryStore((s) => s.browseArtistName);
  const browseYear = useLibraryStore((s) => s.browseYear);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const artists = useLibraryStore((s) => s.artists);
  const selectGenre = useLibraryStore((s) => s.selectGenre);
  const clearSearchResults = useLibraryStore((s) => s.clearSearchResults);
  const flatGenres = useSettingsStore((s) => s.flatGenres);

  const { fontSize, crumbPadH, crumbPadV, crumbGap, barPadH, barPadV, sepSpacing } =
    useBreadcrumbDebugStore();

  // Read sidebarMode at call-time to avoid stale closures.
  const selectAll = useCallback(() => {
    useLibraryStore.setState({ selectedGenreId: "__all__" });
    useLibraryStore.getState().loadAllAlbums();
  }, []);

  const crumbs: Crumb[] = useMemo(() => {
    if (activeBookmarkName) {
      return [{ label: activeBookmarkName }];
    }

    if (searchQuery) {
      return [{ label: searchQuery }];
    }

    if (browseArtistName) {
      const match = artists.find((a) => a.name === browseArtistName);
      const flag = match?.country ? countryToFlag(match.country) : null;
      return [{ label: browseArtistName, flag }];
    }

    if (browseYear) {
      return [{ label: String(browseYear) }];
    }

    if (sidebarMode === "artists" && selectedArtistId) {
      const artist = artists.find((a) => a.sourceId === selectedArtistId);
      const flag = artist?.country ? countryToFlag(artist.country) : null;
      return [{ label: artist?.name ?? "Artist", flag }];
    }

    if (!selectedGenreId || selectedGenreId === "__all__") {
      return [{ label: "All" }];
    }

    if (flatGenres) {
      const node = findNodeById(genreTree, selectedGenreId);
      return [{ label: "All", onClick: selectAll }, { label: node?.name ?? selectedGenreId }];
    }

    // Hierarchical: split the path-based id to build the trail.
    const segments = selectedGenreId.split("/");
    const trail: Crumb[] = [{ label: "All", onClick: selectAll }];

    for (let i = 0; i < segments.length; i++) {
      const partialId = segments.slice(0, i + 1).join("/");
      const node = findNodeById(genreTree, partialId);
      const isLast = i === segments.length - 1;

      if (isLast) {
        trail.push({ label: node?.name ?? segments[i] });
      } else {
        const clickNode = node;
        trail.push({
          label: node?.name ?? segments[i],
          onClick: clickNode ? () => selectGenre(clickNode) : undefined,
        });
      }
    }

    return trail;
  }, [
    activeBookmarkName,
    searchQuery,
    browseArtistName,
    browseYear,
    sidebarMode,
    selectedArtistId,
    artists,
    selectedGenreId,
    flatGenres,
    genreTree,
    selectGenre,
    selectAll,
  ]);

  return (
    <div
      className="breadcrumb-bar"
      style={{
        padding: `${barPadV}px ${barPadH}px`,
        fontSize,
        gap: crumbGap,
      }}
    >
      <div className="breadcrumb-trail" style={{ gap: crumbGap }}>
        {crumbs.map((crumb, i) => {
          const isLast = i === crumbs.length - 1;
          return (
            <span key={i} style={{ display: "inline-flex", alignItems: "center", gap: crumbGap }}>
              {i > 0 && (
                <span className="crumb-sep" style={{ margin: `0 ${sepSpacing}px` }}>
                  &gt;
                </span>
              )}
              <span
                className={`crumb${crumb.onClick && !isLast ? " crumb-link" : ""}`}
                style={{ padding: `${crumbPadV}px ${crumbPadH}px` }}
                onClick={crumb.onClick}
              >
                {crumb.label}
                {crumb.flag && <span className="crumb-flag">{crumb.flag}</span>}
              </span>
            </span>
          );
        })}
        {searchQuery && (
          <button className="crumb-clear" onClick={clearSearchResults} title="Clear search">
            &times;
          </button>
        )}
      </div>
    </div>
  );
}
