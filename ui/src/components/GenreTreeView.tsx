import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { GenreNode } from "../lib/types";
import { useLibraryStore, hasActiveFilters } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useGenreDebugStore } from "./GenreDebugPanel";
import { IconTriangleFilled, IconChevronOpenDown } from "./Icons";

interface FlatRow {
  node: GenreNode;
  depth: number;
  hasChildren: boolean;
  isLastChild: boolean;
  continuationMask: boolean[];
}

function flattenTree(
  nodes: GenreNode[],
  expanded: Set<string>,
  depth = 0,
  mask: boolean[] = [],
): FlatRow[] {
  const rows: FlatRow[] = [];
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    const hasChildren = !!node.children?.length;
    const isLastChild = i === nodes.length - 1;
    rows.push({ node, depth, hasChildren, isLastChild, continuationMask: mask });
    if (hasChildren && expanded.has(node.id)) {
      rows.push(...flattenTree(node.children!, expanded, depth + 1, [...mask, !isLastChild]));
    }
  }
  return rows;
}

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
    .map((node) => ({
      node,
      depth: 0,
      hasChildren: false,
      isLastChild: true,
      continuationMask: [],
    }));
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
  const selectGenreOnly = useLibraryStore((s) => s.selectGenreOnly);
  const loadAllAlbums = useLibraryStore((s) => s.loadAllAlbums);
  const set = useLibraryStore.setState;

  const { chevronSize, chevronWidth, textSize, padH, rowHeight, indentDepth } =
    useGenreDebugStore();
  const libraryPadding = useSettingsStore((s) => s.libraryPadding);
  const flatGenres = useSettingsStore((s) => s.flatGenres);
  const albumFilters = useLibraryStore((s) => s.albumFilters);
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

  useEffect(() => {
    if (parentRef.current) parentRef.current.scrollTop = 0;
  }, [flatGenres]);

  useEffect(() => {
    if (!selectedGenreId || selectedGenreId === "__all__") return;
    const rowIdx = rows.findIndex((r) => r.node.id === selectedGenreId);
    if (rowIdx >= 0) {
      virtualizer.scrollToIndex(rowIdx + 1, { align: "auto" });
    }
  }, [selectedGenreId, rows, virtualizer]);

  if (!genreTree.length) {
    const message = hasActiveFilters(albumFilters)
      ? "No results found, please reduce your filters"
      : "No genres loaded";
    return <div className="empty-state">{message}</div>;
  }

  const rowStyle: React.CSSProperties = {
    position: "absolute",
    left: 0,
    right: 0,
    height: effectiveRowHeight,
    display: "flex",
    alignItems: "center",
    paddingLeft: padH,
    paddingRight: padH,
    fontSize: textSize,
    cursor: "pointer",
    whiteSpace: "nowrap",
  };

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
                style={{ ...rowStyle, top: vItem.start }}
                onClick={() => {
                  set({ selectedGenreId: "__all__" });
                  loadAllAlbums();
                }}
              >
                {flatGenres ? (
                  <span style={{ width: chevronWidth, flexShrink: 0 }} />
                ) : (
                  <span
                    className="genre-chevron-toggle"
                    style={chevronStyle}
                    onClick={(e) => {
                      e.stopPropagation();
                      allExpanded ? collapseAll() : expandAll();
                    }}
                  >
                    {allExpanded ? <IconChevronOpenDown /> : <IconTriangleFilled />}
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
          const showDualCount =
            !flatGenres &&
            row.hasChildren &&
            row.node.albumCount > 0 &&
            row.node.deduplicatedTotalCount > 0 &&
            row.node.albumCount !== row.node.deduplicatedTotalCount;

          return (
            <div
              key={row.node.id}
              className={`genre-row${isSelected ? " selected" : ""}`}
              style={{ ...rowStyle, top: vItem.start }}
              onClick={() => selectGenre(row.node)}
            >
              {!flatGenres &&
                row.depth > 0 &&
                Array.from({ length: row.depth }, (_, i) => {
                  const isElbow = i === row.depth - 1;
                  const continues = isElbow ? !row.isLastChild : row.continuationMask[i + 1];
                  return (
                    <span
                      key={i}
                      className={[
                        "genre-guide",
                        isElbow ? "guide--elbow" : "",
                        continues ? "guide--continues" : "",
                        isElbow && !row.hasChildren ? "guide--leaf" : "",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      style={{ width: indentDepth }}
                    />
                  );
                })}
              {row.hasChildren ? (
                <span
                  className="genre-chevron-toggle"
                  style={chevronStyle}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleGenreExpanded(row.node.id);
                  }}
                >
                  {isExpanded ? <IconChevronOpenDown /> : <IconTriangleFilled />}
                </span>
              ) : (
                <span style={{ width: chevronWidth, flexShrink: 0 }} />
              )}
              <span className="genre-name">{row.node.name}</span>
              {showDualCount ? (
                <>
                  <span
                    className="genre-count genre-count-link"
                    onClick={(e) => {
                      e.stopPropagation();
                      selectGenreOnly(row.node);
                    }}
                    title={`${row.node.name} only`}
                  >
                    {row.node.albumCount}
                  </span>
                  <span className="genre-count genre-count-sep">/</span>
                  <span
                    className="genre-count genre-count-link"
                    onClick={(e) => {
                      e.stopPropagation();
                      selectGenre(row.node);
                    }}
                    title={`${row.node.name} and all children`}
                  >
                    {row.node.deduplicatedTotalCount}
                  </span>
                </>
              ) : (
                (flatGenres ? row.node.albumCount : row.node.deduplicatedTotalCount) > 0 && (
                  <span className="genre-count">
                    {flatGenres ? row.node.albumCount : row.node.deduplicatedTotalCount}
                  </span>
                )
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
