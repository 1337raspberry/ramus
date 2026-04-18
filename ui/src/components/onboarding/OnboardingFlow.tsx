import { useCallback, useEffect, useState } from "react";
import { finalizeOnboarding, connectManualUrl } from "../../lib/commands";
import type { LibrarySection, PlexServer } from "../../lib/types";
import OAuthSignIn from "./OAuthSignIn";
import ServerPicker from "./ServerPicker";
import LibraryPicker from "./LibraryPicker";
import InitialSync from "./InitialSync";

type Step = "signIn" | "discoverServers" | "selectLibrary" | "initialSync";

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
        .then(() => setStep("initialSync"))
        .catch((e) => setFinalizeError(String(e)));
    },
    [server, serverUrl],
  );

  const handleSyncComplete = useCallback(() => {
    clearOnboardingStorage();
    onComplete();
  }, [onComplete]);

  const handleSkip = useCallback(() => {
    clearOnboardingStorage();
    onComplete();
  }, [onComplete]);

  return (
    <div className="onboarding-container">
      <div className="onboarding-card">
        <div className="onboarding-brand">ramus</div>
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
        {step === "initialSync" && (
          <InitialSync onComplete={handleSyncComplete} onSkip={handleSkip} />
        )}
      </div>
    </div>
  );
}
