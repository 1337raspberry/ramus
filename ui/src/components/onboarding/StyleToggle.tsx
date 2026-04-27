import { useEffect, useRef, useState } from "react";
import { getSettings, updateSettings } from "../../lib/commands";
import type { Settings } from "../../lib/types";

interface Props {
  onContinue: () => void;
}

export default function StyleToggle({ onContinue }: Props) {
  const [include, setInclude] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Snapshot settings at mount so the click handler doesn't race other writers.
  const snapshotRef = useRef<Settings | null>(null);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        snapshotRef.current = s;
        setInclude(s.includePlexStyles);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleContinue = async () => {
    if (!snapshotRef.current) {
      onContinue();
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await updateSettings({ ...snapshotRef.current, includePlexStyles: include });
      onContinue();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  return (
    <div className="onboarding-step">
      <h2>Include Plex Style tags?</h2>
      <p className="onboarding-subtitle">
        Plex tags each album with a primary genre and one or more styles (more specific sub-genres).
        Most libraries are richer with both — turn this off if you tag genres yourself and use
        &ldquo;prefer local metadata&rdquo;.
      </p>

      <label className="onboarding-toggle">
        <input
          type="checkbox"
          checked={include}
          onChange={(e) => setInclude(e.target.checked)}
          disabled={saving}
        />
        <span>Merge Style tags into Genres during sync</span>
      </label>

      {error && <div className="onboarding-error">{error}</div>}

      <div className="onboarding-actions">
        <button className="onboarding-primary-btn" onClick={handleContinue} disabled={saving}>
          Continue
        </button>
      </div>
    </div>
  );
}
