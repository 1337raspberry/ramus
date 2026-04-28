import { useLayoutEffect, useMemo, useRef } from "react";
import type { GenreNode } from "../lib/types";
import { useLibraryStore, hasActiveFilters } from "../stores/libraryStore";
import { IconTriangleFilled, IconChevronOpenDown } from "../components/Icons";
import MobileSettingsRow from "./MobileSettingsRow";

let savedScrollTop = 0;

function countExpandable(nodes: GenreNode[]): number {
  let c = 0;
  for (const n of nodes) {
    if (n.children?.length) c += 1 + countExpandable(n.children);
  }
  return c;
}

interface Props {
  onOpenSettings: () => void;
}

interface FlatRow {
  node: GenreNode;
  depth: number;
  hasChildren: boolean;
  isLastChild: boolean;
  continuationMask: boolean[];
}

function flatten(
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
      rows.push(...flatten(node.children!, expanded, depth + 1, [...mask, !isLastChild]));
    }
  }
  return rows;
}

function MobileGenreRow({
  row,
  isOpen,
  onToggleExpand,
}: {
  row: FlatRow;
  isOpen: boolean;
  onToggleExpand: () => void;
}) {
  const selectGenre = useLibraryStore((s) => s.selectGenre);
  const selectGenreOnly = useLibraryStore((s) => s.selectGenreOnly);

  const count = row.node.deduplicatedTotalCount || row.node.albumCount;
  const showDualCount =
    row.hasChildren &&
    row.node.albumCount > 0 &&
    row.node.deduplicatedTotalCount > 0 &&
    row.node.albumCount !== row.node.deduplicatedTotalCount;

  return (
    <div
      data-genre-id={row.node.id}
      className="mobile-genre-row"
      onClick={() => selectGenre(row.node)}
    >
      {row.depth > 0 &&
        Array.from({ length: row.depth }, (_, i) => {
          const isElbow = i === row.depth - 1;
          const continues = isElbow ? !row.isLastChild : row.continuationMask[i + 1];
          return (
            <span
              key={i}
              className={[
                "mobile-genre-guide",
                isElbow ? "guide--elbow" : "",
                continues ? "guide--continues" : "",
                isElbow && !row.hasChildren ? "guide--leaf" : "",
              ]
                .filter(Boolean)
                .join(" ")}
            />
          );
        })}
      {row.hasChildren ? (
        <span
          className="mobile-genre-chev"
          onClick={(e) => {
            e.stopPropagation();
            onToggleExpand();
          }}
        >
          {isOpen ? <IconChevronOpenDown /> : <IconTriangleFilled />}
        </span>
      ) : (
        <span className="mobile-genre-chev-spacer" />
      )}
      <span className="mobile-genre-name">{row.node.name}</span>
      {showDualCount ? (
        <>
          <span
            className="mobile-genre-count mobile-genre-count-link"
            onClick={(e) => {
              e.stopPropagation();
              selectGenre(row.node);
            }}
          >
            {row.node.deduplicatedTotalCount}
          </span>
          <span className="mobile-genre-count mobile-genre-count-sep">|</span>
          <span
            className="mobile-genre-count mobile-genre-count-link mobile-genre-count-secondary"
            onClick={(e) => {
              e.stopPropagation();
              selectGenreOnly(row.node);
            }}
          >
            ({row.node.albumCount})
          </span>
        </>
      ) : (
        count > 0 && <span className="mobile-genre-count">{count}</span>
      )}
    </div>
  );
}

export default function MobileGenreTree({ onOpenSettings }: Props) {
  const genreTree = useLibraryStore((s) => s.genreTree);
  const totalAlbumCount = useLibraryStore((s) => s.totalAlbumCount);
  const expanded = useLibraryStore((s) => s.expandedGenreIds);
  const toggleExpanded = useLibraryStore((s) => s.toggleGenreExpanded);
  const expandAll = useLibraryStore((s) => s.expandAll);
  const collapseAll = useLibraryStore((s) => s.collapseAll);
  const loadAllAlbums = useLibraryStore((s) => s.loadAllAlbums);
  const albumFilters = useLibraryStore((s) => s.albumFilters);

  const scrollRef = useRef<HTMLDivElement>(null);
  const lastSelectedGenreId = useLibraryStore((s) => s.lastSelectedGenreId);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (lastSelectedGenreId) {
      const target = el.querySelector(`[data-genre-id="${CSS.escape(lastSelectedGenreId)}"]`);
      if (target) {
        target.scrollIntoView({ block: "center" });
        return;
      }
    }
    if (savedScrollTop > 0) el.scrollTop = savedScrollTop;
  }, [lastSelectedGenreId]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    return () => {
      if (el) savedScrollTop = el.scrollTop;
    };
  }, []);

  const rows = useMemo(() => flatten(genreTree, expanded), [genreTree, expanded]);
  const allExpanded = useMemo(
    () => !!genreTree.length && expanded.size >= countExpandable(genreTree),
    [genreTree, expanded],
  );

  const handleAll = () => {
    useLibraryStore.setState({ selectedGenreId: "__all__" });
    loadAllAlbums();
  };

  if (!genreTree.length) {
    const message = hasActiveFilters(albumFilters)
      ? "No results found, please reduce your filters"
      : "No genres loaded";
    return <div className="mobile-empty">{message}</div>;
  }

  return (
    <div ref={scrollRef} className="mobile-genre-tree">
      <div className="mobile-genre-row mobile-genre-row-all" onClick={handleAll}>
        <span
          className="mobile-genre-chev"
          onClick={(e) => {
            e.stopPropagation();
            allExpanded ? collapseAll() : expandAll();
          }}
        >
          {allExpanded ? <IconChevronOpenDown /> : <IconTriangleFilled />}
        </span>
        <span className="mobile-genre-name" style={{ fontWeight: 600 }}>
          All
        </span>
        <span className="mobile-genre-count mobile-genre-count-quiet">{totalAlbumCount}</span>
      </div>
      {rows.map((row) => (
        <MobileGenreRow
          key={row.node.id}
          row={row}
          isOpen={expanded.has(row.node.id)}
          onToggleExpand={() => toggleExpanded(row.node.id)}
        />
      ))}
      <MobileSettingsRow onOpen={onOpenSettings} />
    </div>
  );
}
