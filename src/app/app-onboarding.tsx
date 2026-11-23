import type { Dispatch, ReactNode, SetStateAction } from "react";
import { OnboardingFlow } from "../components/onboarding/OnboardingFlow";
import { applyAutostartDefaultOnce } from "../lib/autostart";
import { hasCompletedAnyOnboardingVersion, markOnboardingComplete } from "../lib/onboarding";
import { handleTitlebarPointerDown } from "./app-helpers";

type AppOnboardingOptions = {
  onboardingRequired: boolean;
  setOnboardingDone: Dispatch<SetStateAction<boolean>>;
};

export function renderAppOnboarding({
  onboardingRequired,
  setOnboardingDone,
}: AppOnboardingOptions): ReactNode | null {
  if (!onboardingRequired) return null;
  return (
    <main className="account-gate-shell">
      <div
        className="titlebar-drag"
        aria-hidden
        data-tauri-drag-region
        onPointerDown={handleTitlebarPointerDown}
      />
      <OnboardingFlow
        onComplete={() => {
          // Read before marking complete: marking writes the completion
          // key that distinguishes a fresh install from a wizard replay.
          const firstOnboardingCompletion = !hasCompletedAnyOnboardingVersion();
          markOnboardingComplete();
          // A background assistant only works while it runs; make sure a
          // fresh install starts at login. One-shot and first-run only: a
          // user who later turns the login item off stays off, and wizard
          // replays never re-enroll existing users.
          void applyAutostartDefaultOnce({ firstOnboardingCompletion });
          setOnboardingDone(true);
        }}
      />
    </main>
  );
}
