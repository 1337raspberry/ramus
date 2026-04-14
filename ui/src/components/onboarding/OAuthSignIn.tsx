import { useCallback, useEffect, useRef, useState } from "react";
import { startOauth, pollOauth } from "../../lib/commands";

interface Props {
  onSuccess: () => void;
}

export default function OAuthSignIn({ onSuccess }: Props) {
  const [pinId, setPinId] = useState<number | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const [copied, setCopied] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const startAuth = useCallback(async () => {
    setError(null);
    try {
      const raw = await startOauth();
      // start_oauth returns JSON: { authUrl, pinId, code }
      const data = JSON.parse(raw);
      setPinId(data.pinId);
      setAuthUrl(data.authUrl);
      setPolling(true);

      // Browser is opened from Rust side via open::that()
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const copyUrl = useCallback(async () => {
    if (!authUrl) return;
    await navigator.clipboard.writeText(authUrl);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [authUrl]);

  useEffect(() => {
    if (!polling || pinId === null) return;

    intervalRef.current = setInterval(async () => {
      try {
        const success = await pollOauth(pinId);
        if (success) {
          setPolling(false);
          if (intervalRef.current) clearInterval(intervalRef.current);
          onSuccess();
        }
      } catch (e) {
        // Terminal error from the backend (PIN expired, polling timeout).
        // Stop polling, surface the message, and re-enable the button so
        // the user can start a fresh flow.
        setPolling(false);
        setPinId(null);
        if (intervalRef.current) clearInterval(intervalRef.current);
        setError(String(e));
      }
    }, 2000);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [polling, pinId, onSuccess]);

  return (
    <div className="onboarding-step">
      <div className="onboarding-icon">{"\uD83C\uDFB5"}</div>
      <h2>Welcome to ramus</h2>
      <p className="onboarding-subtitle">Sign in with your Plex account to get started.</p>

      {!polling && (
        <button className="onboarding-primary-btn" onClick={startAuth}>
          Sign in with Plex
        </button>
      )}

      {polling && (
        <div className="onboarding-polling">
          <div className="onboarding-polling-text">
            A sign-in page has been opened in your browser.
          </div>
          <div className="onboarding-polling-subtext">Complete the sign-in there to continue.</div>
          <button className="onboarding-copy-url" onClick={copyUrl}>
            {copied ? "Copied!" : "Wrong browser? Copy link to open manually"}
          </button>
        </div>
      )}

      {error && <div className="onboarding-error">{error}</div>}
    </div>
  );
}
