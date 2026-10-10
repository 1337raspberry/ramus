import { useCallback, useEffect, useRef, useState } from "react";
import { startOauth, pollOauth } from "../../lib/commands";
import ramusLogo from "../../assets/ramus-logo.png";

// Pin state survives a webview reload so polling resumes automatically
// when the user returns from the browser after completing the OAuth
// handshake, instead of demanding a re-click on "Sign in with Plex".
const PIN_STORAGE_KEY = "ramus.onboarding.pin.v1";

// iOS signs in through an in-app sheet that the backend closes once the
// poll finds the token; elsewhere the page opens in the default browser.
const IS_IOS = /iPhone|iPad|iPod/.test(navigator.userAgent);

interface PersistedPin {
  pinId: number;
  authUrl: string;
}

function loadPin(): PersistedPin | null {
  try {
    const raw = localStorage.getItem(PIN_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as PersistedPin;
  } catch {
    return null;
  }
}

function savePin(p: PersistedPin) {
  try {
    localStorage.setItem(PIN_STORAGE_KEY, JSON.stringify(p));
  } catch {}
}

export function clearPin() {
  try {
    localStorage.removeItem(PIN_STORAGE_KEY);
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
  const [starting, setStarting] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // An interval poll and the sheet-closed check can both find the token.
  const signedInRef = useRef(false);

  const startAuth = useCallback(async () => {
    if (starting) return;
    setStarting(true);
    setError(null);
    try {
      const raw = await startOauth();
      // start_oauth returns JSON: { authUrl, pinId }.
      const data = JSON.parse(raw);
      setPinId(data.pinId);
      setAuthUrl(data.authUrl);
      setPolling(true);
      savePin({ pinId: data.pinId, authUrl: data.authUrl });

      // The backend opens the page: the in-app sheet on iOS, the default
      // browser elsewhere.
    } catch (e) {
      setError(String(e));
    } finally {
      setStarting(false);
    }
  }, [starting]);

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
        if (success && !signedInRef.current) {
          signedInRef.current = true;
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

  // The iOS sheet only reports a close the user made (or a sheet that
  // could not be shown). Check once more in case they closed it straight
  // after finishing, otherwise go back to the start.
  useEffect(() => {
    if (!IS_IOS || !polling || pinId === null) return;
    const onClosed = async () => {
      let success = false;
      try {
        success = await pollOauth(pinId);
      } catch {
        // An expired code is the same as a cancel here.
      }
      if (signedInRef.current) return;
      if (intervalRef.current) clearInterval(intervalRef.current);
      setPolling(false);
      clearPin();
      if (success) {
        signedInRef.current = true;
        onSuccess();
      } else {
        setPinId(null);
      }
    };
    window.addEventListener("webAuthClosed", onClosed);
    return () => window.removeEventListener("webAuthClosed", onClosed);
  }, [polling, pinId, onSuccess]);

  return (
    <div className="onboarding-step">
      <img src={ramusLogo} alt="ramus" className="onboarding-logo" />
      <h2>Welcome to ramus</h2>
      <p className="onboarding-subtitle">Sign in with your Plex account to get started.</p>

      {!polling && (
        <button className="onboarding-primary-btn" onClick={startAuth} disabled={starting}>
          {starting ? "Connecting…" : "Sign in with Plex"}
        </button>
      )}

      {polling && !IS_IOS && (
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

      {/* Seen behind the sheet, or after a restart that lost it. */}
      {polling && IS_IOS && (
        <div className="onboarding-polling">
          <div className="onboarding-polling-text">Waiting for Plex sign-in…</div>
          <button className="onboarding-copy-url" onClick={startAuth} disabled={starting}>
            Start again
          </button>
        </div>
      )}

      {error && <div className="onboarding-error">{error}</div>}
    </div>
  );
}
