import { useCallback, useEffect, useState } from "react";
import { finalizeOnboarding, connectManualUrl, logout } from "../../lib/commands";
import type { LibrarySection, PlexServer } from "../../lib/types";
import OAuthSignIn, { clearPin } from "./OAuthSignIn";
import ServerPicker from "./ServerPicker";
import LibraryPicker from "./LibraryPicker";
import StyleToggle from "./StyleToggle";
import InitialSync from "./InitialSync";

type Step = "signIn" | "discoverServers" | "selectLibrary" | "styleToggle" | "initialSync";

// Onboarding progress survives a WKWebView content-process kill via
// localStorage. sessionStorage is lost when iOS terminates the web content
// process under memory pressure (common during OAuth since Safari adds a
// second web process). localStorage persists across process restarts.
const STORAGE_KEY = "ramus.onboarding.v1";

interface PersistedState {
  step: Step;
  server: PlexServer | null;
  serverUrl: string;
}

function loadPersisted(): PersistedState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as PersistedState;
  } catch {
    return null;
  }
}

function savePersisted(state: PersistedState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {}
}

export function clearOnboardingStorage() {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {}
}

interface Props {
  onComplete: () => void;
}

export default function OnboardingFlow({ onComplete }: Props) {
  const persisted = loadPersisted();
  const [step, setStep] = useState<Step>(persisted?.step ?? "signIn");
  const [server, setServer] = useState<PlexServer | null>(persisted?.server ?? null);
  const [serverUrl, setServerUrl] = useState<string>(persisted?.serverUrl ?? "");
  const [finalizeError, setFinalizeError] = useState<string | null>(null);

  useEffect(() => {
    savePersisted({ step, server, serverUrl });
  }, [step, server, serverUrl]);

  const handleSignInSuccess = useCallback(() => {
    setStep("discoverServers");
  }, []);

  const handleServerSelect = useCallback(async (srv: PlexServer, url: string) => {
    setServer(srv);
    setServerUrl(url);
    // Pre-connect so findMusicLibraries works; finalize also connects as a fallback.
    try {
      await connectManualUrl(url);
    } catch {}
    setStep("selectLibrary");
  }, []);

  const handleLibrarySelect = useCallback(
    (lib: LibrarySection) => {
      if (!server) return;
      setFinalizeError(null);
      finalizeOnboarding(server.machineIdentifier, lib.key, serverUrl)
        .then(() => setStep("styleToggle"))
        .catch((e) => setFinalizeError(String(e)));
    },
    [server, serverUrl],
  );

  const handleStyleContinue = useCallback(() => {
    setStep("initialSync");
  }, []);

  const handleSyncComplete = useCallback(() => {
    clearOnboardingStorage();
    onComplete();
  }, [onComplete]);

  const handleSkip = useCallback(() => {
    clearOnboardingStorage();
    onComplete();
  }, [onComplete]);

  // Escape hatch for users stuck in a broken mid-flow state (e.g. quit
  // at the library picker, came back to "not connected to a Plex
  // server"). Wipes both the Rust-side credentials and the JS-side step
  // persistence so the next mount lands cleanly on step one.
  const handleRestart = useCallback(() => {
    logout().catch(() => {
      // Best-effort — even if token-store wipe fails, the local state
      // reset below still drops the user back to the sign-in screen.
    });
    clearOnboardingStorage();
    clearPin();
    setServer(null);
    setServerUrl("");
    setFinalizeError(null);
    setStep("signIn");
  }, []);

  return (
    <div className="onboarding-container">
      <div className="onboarding-card">
        {step === "signIn" && <OAuthSignIn onSuccess={handleSignInSuccess} />}
        {step === "discoverServers" && <ServerPicker onSelect={handleServerSelect} />}
        {step === "selectLibrary" && server && (
          <>
            <LibraryPicker server={server} onSelect={handleLibrarySelect} />
            {finalizeError && (
              <div className="onboarding-error">Couldn't finish setup: {finalizeError}</div>
            )}
          </>
        )}
        {step === "styleToggle" && <StyleToggle onContinue={handleStyleContinue} />}
        {step === "initialSync" && (
          <InitialSync onComplete={handleSyncComplete} onSkip={handleSkip} />
        )}
        <div className="onboarding-restart">
          <button className="onboarding-text-btn" onClick={handleRestart}>
            Restart setup
          </button>
        </div>
      </div>
    </div>
  );
}
