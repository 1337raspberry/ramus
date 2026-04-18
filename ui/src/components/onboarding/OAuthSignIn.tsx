import { useCallback, useEffect, useRef, useState } from "react";
import { startOauth, pollOauth } from "../../lib/commands";

// Pin state survives a WKWebView reload so polling resumes automatically
// when the user returns from Safari after completing the OAuth handshake,
// instead of demanding a re-click on "Sign in with Plex".
const PIN_STORAGE_KEY = "ramus.onboarding.pin.v1";

interface PersistedPin {
  pinId: number;
  authUrl: string;
}

function loadPin(): PersistedPin | null {
  try {
    const raw = sessionStorage.getItem(PIN_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as PersistedPin;
  } catch {
    return null;
  }
}

function savePin(p: PersistedPin) {
  try {
    sessionStorage.setItem(PIN_STORAGE_KEY, JSON.stringify(p));
  } catch {}
}

function clearPin() {
  try {
    sessionStorage.removeItem(PIN_STORAGE_KEY);
  } catch {}
}

interface Props {
  onSuccess: () => void;
}

export default function OAuthSignIn({ onSuccess }: Props) {
  const stored = loadPin();
  const [pinId, setPinId] = useState<number | null>(stored?.pinId ?? null);
  const [authUrl, setAuthUrl] = useState<string | null>(stored?.authUrl ?? null);
  const [error, setError] = useState<string | null>(null);
  const [polling, setPolling] = useState(stored !== null);
  const [copied, setCopied] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const startAuth = useCallback(async () => {
    setError(null);
    try {
      const raw = await startOauth();
      // start_oauth returns JSON: { authUrl, pinId }.
      const data = JSON.parse(raw);
      setPinId(data.pinId);
      setAuthUrl(data.authUrl);
      setPolling(true);
      savePin({ pinId: data.pinId, authUrl: data.authUrl });

      // Browser opens from Rust via tauri-plugin-opener (routes through
      // the OS: NSWorkspace / UIApplication / ShellExecuteW / xdg-open).
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
          clearPin();
          if (intervalRef.current) clearInterval(intervalRef.current);
          onSuccess();
        }
      } catch (e) {
        // Terminal backend error (PIN expired, polling timeout). Stop
        // polling, surface the message, and re-enable the button for a
        // fresh flow.
        setPolling(false);
        setPinId(null);
        clearPin();
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
