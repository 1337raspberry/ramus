import { useLibraryStore } from "../stores/libraryStore";
import { IconChevronRight } from "../components/Icons";
import MobileSettingsRow from "./MobileSettingsRow";

interface Props {
  onOpenSettings: () => void;
}

export default function MobileArtistList({ onOpenSettings }: Props) {
  const artists = useLibraryStore((s) => s.artists);
  const selectArtist = useLibraryStore((s) => s.selectArtist);

  return (
    <div className="mobile-artist-list">
      {artists.length === 0 ? (
        <div className="mobile-empty">No artists loaded</div>
      ) : (
        artists.map((a) => (
          <button
            key={a.sourceId}
            className="mobile-artist-row"
            onClick={() => selectArtist(a.sourceId)}
          >
            <span className="mobile-artist-name">{a.name}</span>
            <IconChevronRight />
          </button>
        ))
      )}
      <MobileSettingsRow onOpen={onOpenSettings} />
    </div>
  );
}
