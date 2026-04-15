import { useEffect, useState } from "react";
import { getAcknowledgementsText } from "../lib/commands";
import type { AcknowledgementsText } from "../lib/types";

interface Props {
  onDismiss: () => void;
}

export default function AcknowledgementsPanel({ onDismiss }: Props) {
  const [text, setText] = useState<AcknowledgementsText | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAcknowledgementsText()
      .then(setText)
      .catch((e) => setError(String(e)));
  }, []);

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
            ramus is released under the MIT License. It builds on libmpv (LGPL-2.1-or-later),
            symphonia and other MPL-2.0 components, the beets project's genre hierarchy (MIT), and
            several hundred other open-source libraries. Full license texts below.
          </p>

          {error && <div className="settings-error">{error}</div>}

          {!text && !error && <div className="acknowledgements-loading">Loading…</div>}

          {text && (
            <pre className="acknowledgements-text">
              {`=== ramus (MIT) ===\n\n${text.mitLicense}\n\n` +
                `=== Attributions (NOTICE) ===\n\n${text.notice}\n\n` +
                `=== libmpv (LGPL-2.1) ===\n\n${text.lgpl}\n\n` +
                `=== symphonia and other MPL-2.0 components ===\n\n${text.mpl}\n\n` +
                `=== Full third-party dependency list ===\n\n${text.thirdParty}\n`}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
