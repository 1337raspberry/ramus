import { useEffect, useState } from "react";
import { findMusicLibraries } from "../../lib/commands";
import type { LibrarySection, PlexServer } from "../../lib/types";

interface Props {
  server: PlexServer;
  onSelect: (library: LibrarySection) => void;
}

export default function LibraryPicker({ server, onSelect }: Props) {
  const [libraries, setLibraries] = useState<LibrarySection[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);

  useEffect(() => {
    findMusicLibraries()
      .then((libs) => {
        setLibraries(libs);
        setLoading(false);
      })
      .catch(() => setLoading(false));
  }, []);

  const handleSelect = (lib: LibrarySection) => {
    setSelectedKey(lib.key);
    onSelect(lib);
  };

  return (
    <div className="onboarding-step">
      <h2>Select a Library</h2>
      <p className="onboarding-subtitle">{server.name}</p>

      {loading && <div className="onboarding-loading">Loading libraries...</div>}

      <div className="library-list">
        {libraries.map((lib) => (
          <div
            key={lib.key}
            className={`library-row${selectedKey === lib.key ? " selected" : ""}`}
            onClick={() => handleSelect(lib)}
          >
            <span className="library-icon">{"\uD83C\uDFB5"}</span>
            <span className="library-name">{lib.title}</span>
            {selectedKey === lib.key && (
              <span className="library-check">{"\u2713"}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
