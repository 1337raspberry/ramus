import { useCallback, useState } from "react";
import { finalizeOnboarding, connectManualUrl } from "../../lib/commands";
import type { LibrarySection, PlexServer } from "../../lib/types";
import OAuthSignIn from "./OAuthSignIn";
import ServerPicker from "./ServerPicker";
import LibraryPicker from "./LibraryPicker";
import InitialSync from "./InitialSync";

type Step = "signIn" | "discoverServers" | "selectLibrary" | "initialSync";

interface Props {
  onComplete: () => void;
}

export default function OnboardingFlow({ onComplete }: Props) {
  const [step, setStep] = useState<Step>("signIn");
  const [server, setServer] = useState<PlexServer | null>(null);
  const [serverUrl, setServerUrl] = useState<string>("");
  const [finalizeError, setFinalizeError] = useState<string | null>(null);

  const handleSignInSuccess = useCallback(() => {
    setStep("discoverServers");
  }, []);

  const handleServerSelect = useCallback(async (srv: PlexServer, url: string) => {
    setServer(srv);
    setServerUrl(url);
    // Pre-connect so findMusicLibraries works; finalize also connects as a fallback
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
    onComplete();
  }, [onComplete]);

  const handleSkip = useCallback(() => {
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
