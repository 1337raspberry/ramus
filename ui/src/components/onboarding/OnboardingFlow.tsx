import { useCallback, useState } from "react";
import { finalizeOnboarding } from "../../lib/commands";
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

  const handleSignInSuccess = useCallback(() => {
    setStep("discoverServers");
  }, []);

  const handleServerSelect = useCallback((srv: PlexServer, url: string) => {
    setServer(srv);
    setServerUrl(url);
    setStep("selectLibrary");
  }, []);

  const handleLibrarySelect = useCallback(
    (lib: LibrarySection) => {
      if (!server) return;
      // Finalize onboarding (sets up cache, sync engine, etc.)
      finalizeOnboarding(server, lib.key, serverUrl)
        .then(() => setStep("initialSync"))
        .catch(() => {});
    },
    [server, serverUrl]
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
        {step === "discoverServers" && (
          <ServerPicker onSelect={handleServerSelect} />
        )}
        {step === "selectLibrary" && server && (
          <LibraryPicker server={server} onSelect={handleLibrarySelect} />
        )}
        {step === "initialSync" && (
          <InitialSync onComplete={handleSyncComplete} onSkip={handleSkip} />
        )}
      </div>
    </div>
  );
}
