import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { GenreNode } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useGenreDebugStore } from "./GenreDebugPanel";
import { IconChevronRight } from "./Icons";

interface FlatRow {
  node: GenreNode;
  depth: number;
  hasChildren: boolean;
}

function flattenTree(nodes: GenreNode[], expanded: Set<string>, depth = 0): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const node of nodes) {
    const hasChildren = !!node.children?.length;
    rows.push({ node, depth, hasChildren });
    if (hasChildren && expanded.has(node.id)) {
      rows.push(...flattenTree(node.children!, expanded, depth + 1));
    }
  }
  return rows;
}

/** Flatten the full tree into an A-Z list of every genre that has albums. */
function flattenToAZ(nodes: GenreNode[]): FlatRow[] {
  const byName = new Map<string, GenreNode>();
  const collect = (list: GenreNode[]) => {
    for (const n of list) {
      if (n.albumCount > 0) {
        const key = n.name.toLowerCase();
        const existing = byName.get(key);
        if (!existing || n.albumCount > existing.albumCount) {
          byName.set(key, n);
        }
      }
      if (n.children) collect(n.children);
    }
  };
  collect(nodes);
  return [...byName.values()]
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((node) => ({ node, depth: 0, hasChildren: false }));
}

export default function GenreTreeView() {
  const parentRef = useRef<HTMLDivElement>(null);

  const genreTree = useLibraryStore((s) => s.genreTree);
  const totalAlbumCount = useLibraryStore((s) => s.totalAlbumCount);
  const expandedGenreIds = useLibraryStore((s) => s.expandedGenreIds);
  const selectedGenreId = useLibraryStore((s) => s.selectedGenreId);
  const toggleGenreExpanded = useLibraryStore((s) => s.toggleGenreExpanded);
  const expandAll = useLibraryStore((s) => s.expandAll);
  const collapseAll = useLibraryStore((s) => s.collapseAll);
  const selectGenre = useLibraryStore((s) => s.selectGenre);
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const loadAllAlbums = useLibraryStore((s) => s.loadAllAlbums);
  const loadFavouriteAlbums = useLibraryStore((s) => s.loadFavouriteAlbums);
  const set = useLibraryStore.setState;

  const { chevronSize, chevronWidth, textSize, padH, rowHeight, indentDepth } =
    useGenreDebugStore();
  const libraryPadding = useSettingsStore((s) => s.libraryPadding);
  const flatGenres = useSettingsStore((s) => s.flatGenres);
  const effectiveRowHeight = Math.max(12, rowHeight + libraryPadding * 2);

  const rows = useMemo(
    () => (flatGenres ? flattenToAZ(genreTree) : flattenTree(genreTree, expandedGenreIds)),
    [genreTree, expandedGenreIds, flatGenres],
  );

  const allExpanded = useMemo(() => {
    if (!genreTree.length) return false;
    const countExpandable = (nodes: GenreNode[]): number => {
      let c = 0;
      for (const n of nodes) {
        if (n.children?.length) {
          c += 1 + countExpandable(n.children);
        }
      }
      return c;
    };
    return expandedGenreIds.size >= countExpandable(genreTree);
  }, [genreTree, expandedGenreIds]);

  const virtualizer = useVirtualizer({
    count: rows.length + 1,
    getScrollElement: () => parentRef.current,
    estimateSize: () => effectiveRowHeight,
    overscan: 20,
  });

  useEffect(() => {
    virtualizer.measure();
  }, [effectiveRowHeight, virtualizer]);

  // Reset scroll when switching between flat and hierarchical mode
  useEffect(() => {
    if (parentRef.current) parentRef.current.scrollTop = 0;
  }, [flatGenres]);

  if (!genreTree.length) {
    return <div className="empty-state">No genres loaded</div>;
  }

  const rowStyle = (depth: number): React.CSSProperties => ({
    position: "absolute",
    left: 0,
    right: 0,
    height: effectiveRowHeight,
    display: "flex",
    alignItems: "center",
    paddingLeft: padH + depth * indentDepth,
    paddingRight: padH,
    fontSize: textSize,
    cursor: "pointer",
    whiteSpace: "nowrap",
    overflow: "hidden",
  });

  const chevronStyle: React.CSSProperties = {
    width: chevronWidth,
    flexShrink: 0,
    fontSize: chevronSize,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
  };

  return (
    <div ref={parentRef} style={{ height: "100%", overflow: "auto", paddingTop: 2 }}>
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((vItem) => {
          if (vItem.index === 0) {
            return (
              <div
                key="__all__"
                className={`genre-row${selectedGenreId === "__all__" ? " selected" : ""}`}
                style={{ ...rowStyle(0), top: vItem.start }}
                onClick={() => {
                  set({ selectedGenreId: "__all__" });
                  if (sidebarMode === "favourites") {
                    loadFavouriteAlbums();
                  } else {
                    loadAllAlbums();
                  }
                }}
              >
                {flatGenres ? (
                  <span style={{ width: chevronWidth, flexShrink: 0 }} />
                ) : (
                  <span
                    className={`genre-chevron${allExpanded ? " expanded" : ""}`}
                    style={chevronStyle}
                    onClick={(e) => {
                      e.stopPropagation();
                      allExpanded ? collapseAll() : expandAll();
                    }}
                  >
                    <IconChevronRight />
                  </span>
                )}
                <span className="genre-name" style={{ fontWeight: 600 }}>
                  All
                </span>
                <span className="genre-count">{totalAlbumCount}</span>
              </div>
            );
          }

          const row = rows[vItem.index - 1];
          const isExpanded = expandedGenreIds.has(row.node.id);
          const isSelected = selectedGenreId === row.node.id;

          return (
            <div
              key={row.node.id}
              className={`genre-row${isSelected ? " selected" : ""}`}
              style={{ ...rowStyle(row.depth), top: vItem.start }}
              onClick={() => selectGenre(row.node)}
            >
              {row.hasChildren ? (
                <span
                  className={`genre-chevron${isExpanded ? " expanded" : ""}`}
                  style={chevronStyle}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleGenreExpanded(row.node.id);
                  }}
                >
                  <IconChevronRight />
                </span>
              ) : (
                <span style={{ width: chevronWidth, flexShrink: 0 }} />
              )}
              <span className="genre-name">{row.node.name}</span>
              {(flatGenres ? row.node.albumCount : row.node.deduplicatedTotalCount) > 0 && (
                <span className="genre-count">
                  {flatGenres ? row.node.albumCount : row.node.deduplicatedTotalCount}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
