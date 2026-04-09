interface Props {
  genres: string[];
  onGenreClick?: (genre: string) => void;
}

export default function FlowLayout({ genres, onGenreClick }: Props) {
  if (!genres.length) return null;

  return (
    <div className="flow-layout">
      {genres.map((genre) => (
        <button key={genre} className="genre-pill" onClick={() => onGenreClick?.(genre)}>
          {genre}
        </button>
      ))}
    </div>
  );
}
