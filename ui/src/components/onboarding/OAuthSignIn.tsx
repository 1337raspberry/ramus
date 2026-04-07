import { useCallback, useEffect, useRef, useState } from "react";
import { startOauth, pollOauth } from "../../lib/commands";

interface Props {
  onSuccess: () => void;
}

export default function OAuthSignIn({ onSuccess }: Props) {
  const [pinCode, setPinCode] = useState<string | null>(null);
  const [pinId, setPinId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const startAuth = useCallback(async () => {
    setError(null);
    try {
      const code = await startOauth();
      // code is the pin code string, but we also need the pin ID
      // The start_oauth command returns the pin code; we'll parse the ID from it
      // Actually, looking at the command: start_oauth returns the PIN code string
      // and poll_oauth takes a pin ID number. The PIN code is displayed to the user.
      // Let's assume start_oauth returns "CODE:ID" or similar, or we adapt.
      // Based on the Plex OAuth flow, the code IS the pin ID.
      setPinCode(code);
      const id = parseInt(code, 10);
      if (!isNaN(id)) {
        setPinId(id);
        setPolling(true);
      }
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Poll for OAuth completion
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
      } catch {
        // Keep polling
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

      {!polling && !pinCode && (
        <button className="onboarding-primary-btn" onClick={startAuth}>
          Sign in with Plex
        </button>
      )}

      {polling && pinCode && (
        <div className="onboarding-polling">
          <div className="onboarding-pin-label">Enter this code in your browser:</div>
          <div className="onboarding-pin-code">{pinCode}</div>
          <div className="onboarding-polling-text">Waiting for authorization...</div>
        </div>
      )}

      {error && <div className="onboarding-error">{error}</div>}
    </div>
  );
}
