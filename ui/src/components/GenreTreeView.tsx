import { useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { GenreNode } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";

interface FlatRow {
  node: GenreNode;
  depth: number;
  hasChildren: boolean;
}

function flattenTree(
  nodes: GenreNode[],
  expanded: Set<string>,
  depth = 0
): FlatRow[] {
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

export default function GenreTreeView() {
  const parentRef = useRef<HTMLDivElement>(null);

  const genreTree = useLibraryStore((s) => s.genreTree);
  const expandedGenreIds = useLibraryStore((s) => s.expandedGenreIds);
  const selectedGenreId = useLibraryStore((s) => s.selectedGenreId);
  const toggleGenreExpanded = useLibraryStore((s) => s.toggleGenreExpanded);
  const expandAll = useLibraryStore((s) => s.expandAll);
  const collapseAll = useLibraryStore((s) => s.collapseAll);
  const selectGenre = useLibraryStore((s) => s.selectGenre);

  const totalCount = useMemo(() => {
    let sum = 0;
    for (const n of genreTree) sum += n.deduplicatedTotalCount;
    return sum;
  }, [genreTree]);

  // "All" sentinel + flattened tree
  const rows = useMemo(
    () => flattenTree(genreTree, expandedGenreIds),
    [genreTree, expandedGenreIds]
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
    count: rows.length + 1, // +1 for "All" row
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 20,
  });

  if (!genreTree.length) {
    return <div className="empty-state">No genres loaded</div>;
  }

  return (
    <div ref={parentRef} style={{ height: "100%", overflow: "auto" }}>
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((vItem) => {
          // First row is the "All" sentinel
          if (vItem.index === 0) {
            return (
              <div
                key="__all__"
                className="genre-row"
                style={{
                  position: "absolute",
                  top: vItem.start,
                  left: 0,
                  right: 0,
                  height: vItem.size,
                }}
                onClick={() => (allExpanded ? collapseAll() : expandAll())}
              >
                <span className="genre-chevron">
                  {allExpanded ? "▾" : "▸"}
                </span>
                <span className="genre-name" style={{ fontWeight: 600 }}>
                  All
                </span>
                <span className="genre-count">{totalCount}</span>
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
              style={{
                position: "absolute",
                top: vItem.start,
                left: 0,
                right: 0,
                height: vItem.size,
                paddingLeft: 8 + row.depth * 8,
              }}
              onClick={() => selectGenre(row.node)}
            >
              {row.hasChildren ? (
                <span
                  className="genre-chevron"
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleGenreExpanded(row.node.id);
                  }}
                >
                  {isExpanded ? "▾" : "▸"}
                </span>
              ) : (
                <span className="genre-chevron-spacer" />
              )}
              <span className="genre-name">{row.node.name}</span>
              {row.node.deduplicatedTotalCount > 0 && (
                <span className="genre-count">
                  {row.node.deduplicatedTotalCount}
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
