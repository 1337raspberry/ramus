import { useMemo } from "react";
import type { GenreNode } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { IconChevronRight, IconChevronDown } from "../components/Icons";
import MobileSettingsRow from "./MobileSettingsRow";

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
}

function flatten(nodes: GenreNode[], expanded: Set<string>, depth = 0): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const node of nodes) {
    const hasChildren = !!node.children?.length;
    rows.push({ node, depth, hasChildren });
    if (hasChildren && expanded.has(node.id)) {
      rows.push(...flatten(node.children!, expanded, depth + 1));
    }
  }
  return rows;
}

export default function MobileGenreTree({ onOpenSettings }: Props) {
  const genreTree = useLibraryStore((s) => s.genreTree);
  const totalAlbumCount = useLibraryStore((s) => s.totalAlbumCount);
  const expanded = useLibraryStore((s) => s.expandedGenreIds);
  const toggleExpanded = useLibraryStore((s) => s.toggleGenreExpanded);
  const expandAll = useLibraryStore((s) => s.expandAll);
  const collapseAll = useLibraryStore((s) => s.collapseAll);
  const selectGenre = useLibraryStore((s) => s.selectGenre);
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const loadAllAlbums = useLibraryStore((s) => s.loadAllAlbums);
  const loadFavouriteAlbums = useLibraryStore((s) => s.loadFavouriteAlbums);

  const rows = useMemo(() => flatten(genreTree, expanded), [genreTree, expanded]);
  const allExpanded = useMemo(
    () => !!genreTree.length && expanded.size >= countExpandable(genreTree),
    [genreTree, expanded],
  );

  const handleAll = () => {
    useLibraryStore.setState({ selectedGenreId: "__all__" });
    if (sidebarMode === "favourites") loadFavouriteAlbums();
    else loadAllAlbums();
  };

  if (!genreTree.length) {
    return <div className="mobile-empty">No genres loaded</div>;
  }

  return (
    <div className="mobile-genre-tree">
      <div className="mobile-genre-row" onClick={handleAll}>
        <span className="mobile-genre-name">All</span>
        <span className="mobile-genre-count">{totalAlbumCount}</span>
        <span
          className="mobile-genre-chev"
          onClick={(e) => {
            e.stopPropagation();
            allExpanded ? collapseAll() : expandAll();
          }}
        >
          {allExpanded ? <IconChevronDown /> : <IconChevronRight />}
        </span>
      </div>
      {rows.map((row) => {
        const isOpen = expanded.has(row.node.id);
        const count = row.node.deduplicatedTotalCount || row.node.albumCount;
        return (
          <div
            key={row.node.id}
            className="mobile-genre-row"
            style={{ paddingLeft: 20 + row.depth * 24 }}
            onClick={() => selectGenre(row.node)}
          >
            <span className="mobile-genre-name">{row.node.name}</span>
            {count > 0 && <span className="mobile-genre-count">{count}</span>}
            <span
              className="mobile-genre-chev"
              onClick={(e) => {
                if (!row.hasChildren) return;
                e.stopPropagation();
                toggleExpanded(row.node.id);
              }}
            >
              {row.hasChildren ? isOpen ? <IconChevronDown /> : <IconChevronRight /> : null}
            </span>
          </div>
        );
      })}
      <MobileSettingsRow onOpen={onOpenSettings} />
    </div>
  );
}
