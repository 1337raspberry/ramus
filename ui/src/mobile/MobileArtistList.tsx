import { useCallback, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore } from "../stores/libraryStore";
import { IconChevronRight } from "../components/Icons";
import { countryToFlag } from "../lib/countryFlag";
import MobileSettingsRow from "./MobileSettingsRow";

interface Props {
  onOpenSettings: () => void;
}

const ROW_HEIGHT = 49;

export default function MobileArtistList({ onOpenSettings }: Props) {
  const artists = useLibraryStore((s) => s.artists);
  const selectArtist = useLibraryStore((s) => s.selectArtist);

  const parentRef = useRef<HTMLDivElement>(null);
  const estimateSize = useCallback(() => ROW_HEIGHT, []);
  const virtualizer = useVirtualizer({
    count: artists.length,
    getScrollElement: () => parentRef.current,
    estimateSize,
    overscan: 8,
  });

  if (artists.length === 0) {
    return (
      <div className="mobile-artist-list">
        <div className="mobile-empty">No artists loaded</div>
        <MobileSettingsRow onOpen={onOpenSettings} />
      </div>
    );
  }

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div ref={parentRef} className="mobile-artist-list">
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualItems.map((vItem) => {
          const a = artists[vItem.index];
          return (
            <button
              key={a.sourceId}
              className="mobile-artist-row"
              style={{
                position: "absolute",
                top: vItem.start,
                left: 0,
                right: 0,
                height: ROW_HEIGHT,
              }}
              onClick={() => selectArtist(a.sourceId)}
            >
              <span className="mobile-artist-flag">
                {a.country ? (countryToFlag(a.country) ?? "") : ""}
              </span>
              <span className="mobile-artist-name">{a.name}</span>
              <IconChevronRight />
            </button>
          );
        })}
      </div>
      <MobileSettingsRow onOpen={onOpenSettings} />
    </div>
  );
}
