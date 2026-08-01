import { genreSurfaceHandlers } from "../lib/genreSurface";
import { useLongPress } from "../lib/useLongPress";

interface Props {
  genres: string[];
  onGenreClick?: (genre: string) => void;
  /**
   * Optional long-press handler. When provided, holding a pill (without
   * scrolling away) fires this instead of the tap. Used on mobile to open the
   * genre-info popover; desktop omits it and pills behave as plain links.
   */
  onGenreLongPress?: (genre: string) => void;
}

export default function FlowLayout({ genres, onGenreClick, onGenreLongPress }: Props) {
  if (!genres.length) return null;

  return (
    <div className="flow-layout">
      {genres.map((genre) => (
        <GenrePill
          key={genre}
          genre={genre}
          onGenreClick={onGenreClick}
          onGenreLongPress={onGenreLongPress}
        />
      ))}
    </div>
  );
}

interface PillProps {
  genre: string;
  onGenreClick?: (genre: string) => void;
  onGenreLongPress?: (genre: string) => void;
}

function GenrePill({ genre, onGenreClick, onGenreLongPress }: PillProps) {
  const longPress = useLongPress({
    onLongPress: () => onGenreLongPress?.(genre),
    onClick: () => onGenreClick?.(genre),
  });

  // Without a long-press handler (desktop), skip the touch timers and use
  // the desktop genre-surface affordances instead: dwell-hover summary card
  // and right-click for the full info modal.
  if (!onGenreLongPress) {
    return (
      <button
        className="genre-pill"
        onClick={() => onGenreClick?.(genre)}
        {...genreSurfaceHandlers(genre)}
      >
        {genre}
      </button>
    );
  }

  return (
    <button className="genre-pill" {...longPress}>
      {genre}
    </button>
  );
}
