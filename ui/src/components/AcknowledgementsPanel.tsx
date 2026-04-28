import { useEffect } from "react";
import { openExternalUrl } from "../lib/commands";

interface Props {
  onDismiss: () => void;
}

const REPO_BASE = "https://github.com/1337raspberry/ramus-xplat/blob/main";
const THIRD_PARTY_URL = `${REPO_BASE}/THIRD_PARTY_LICENSES.md`;
const NOTICE_URL = `${REPO_BASE}/NOTICE.md`;
const LICENSE_URL = `${REPO_BASE}/LICENSE`;

interface KeyComponent {
  name: string;
  license: string;
  description: string;
}

const KEY_COMPONENTS: KeyComponent[] = [
  {
    name: "libmpv",
    license: "LGPL-2.1-or-later",
    description: "Audio playback engine. Loaded dynamically at runtime; user-swappable.",
  },
  {
    name: "symphonia",
    license: "MPL-2.0",
    description: "Pure-Rust audio decoders, used for the focus-mode spectrum visualiser.",
  },
  {
    name: "Genre tree",
    license: "MIT",
    description:
      "Initially based on the beets project's genre hierarchy; substantially extended and restructured since.",
  },
  {
    name: "Tauri",
    license: "MIT / Apache-2.0",
    description: "Cross-platform app runtime.",
  },
  {
    name: "React, Zustand, @tanstack/react-virtual",
    license: "MIT",
    description: "Frontend framework, state, virtualised lists.",
  },
  {
    name: "rusqlite, reqwest, serde, tokio",
    license: "MIT / Apache-2.0",
    description: "SQLite, HTTP, serialisation, async runtime.",
  },
];

export default function AcknowledgementsPanel({ onDismiss }: Props) {
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onDismiss]);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  const openLink = (url: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    openExternalUrl(url).catch(() => {
      /* swallow — the link is informational, no recovery action */
    });
  };

  return (
    <div className="settings-backdrop" onClick={handleBackdropClick}>
      <div className="settings-panel acknowledgements-panel glass">
        <div className="settings-header">
          <h2>Acknowledgements</h2>
          <button className="settings-close" onClick={onDismiss}>
            x
          </button>
        </div>

        <div className="acknowledgements-body">
          <p className="acknowledgements-summary">
            ramus is released under the{" "}
            <a href={LICENSE_URL} onClick={openLink(LICENSE_URL)}>
              MIT License
            </a>
            . It builds on a number of open-source projects, the most prominent of which are
            credited below. The full list of bundled Rust crates and npm packages, with license
            texts, is generated from the lockfiles and ships with every build.
          </p>

          <div className="acknowledgements-section">
            <h3>Key components</h3>
            <ul className="acknowledgements-list">
              {KEY_COMPONENTS.map((c) => (
                <li key={c.name}>
                  <div className="acknowledgements-list-head">
                    <span className="acknowledgements-list-name">{c.name}</span>
                    <span className="acknowledgements-list-license">{c.license}</span>
                  </div>
                  <div className="acknowledgements-list-desc">{c.description}</div>
                </li>
              ))}
            </ul>
          </div>

          <div className="acknowledgements-section">
            <h3>Full third-party license list</h3>
            <p className="acknowledgements-section-body">
              The complete dependency manifest with full license texts is regenerated from
              <code> Cargo.lock</code> and <code>ui/package-lock.json</code> on every release; CI
              fails the build if it drifts. The same files are bundled with the installed app for
              offline reference.
            </p>
            <div className="acknowledgements-links">
              <a href={THIRD_PARTY_URL} onClick={openLink(THIRD_PARTY_URL)}>
                THIRD_PARTY_LICENSES.md
              </a>
              <a href={NOTICE_URL} onClick={openLink(NOTICE_URL)}>
                NOTICE.md
              </a>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
