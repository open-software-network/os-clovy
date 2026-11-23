import { IconChevronLeftSmall } from "central-icons/IconChevronLeftSmall";
import { useEffect, useState } from "react";
import { useT } from "../../i18n";
import {
  onboardingArea,
  onboardingResumeStep,
  setOnboardingResumeStep,
  type OnboardingArea,
} from "../../lib/onboarding";
import { fallbackDictationCapabilities } from "../../lib/platform";
import { dictationSettings, setDictationShortcut } from "../../lib/tauri";
import type { DictationShortcutSetting } from "../../lib/tauri";
import { PermissionsStep } from "./steps/PermissionSteps";
import { AreaStep } from "./steps/AreaStep";
import { usePermissionStatuses, useSystemAudioStatus } from "./use-permission-status";

type StepId = "area" | "permissions";

// The product default: bare fn, mirroring DictationShortcutSetting::bare_fn()
// on the Rust side.
const FN_SHORTCUT = {
  code: "Fn",
  modifiers: {
    command: false,
    control: false,
    option: false,
    shift: false,
    function: true,
  },
  label: "Fn",
  pressCount: 1 as const,
};

// Mirrors DictationShortcutSetting::control_option_d() on the Rust side: the
// factory default a fresh install carries before anyone has touched it.
function isFactoryDefaultShortcut(shortcut: DictationShortcutSetting) {
  return (
    shortcut.code === "KeyD" &&
    shortcut.modifiers.control &&
    shortcut.modifiers.option &&
    !shortcut.modifiers.command &&
    !shortcut.modifiers.shift &&
    !shortcut.modifiers.function
  );
}

const ONBOARDING_STEPS: StepId[] = ["area", "permissions"];

type Props = {
  onComplete: () => void;
};

function initialStepIndex(steps: StepId[]): number {
  const demoStep = browserOnboardingDemoStep();
  if (demoStep) {
    const demoIndex = steps.indexOf(demoStep);
    if (demoIndex !== -1) return demoIndex;
  }
  const saved = onboardingResumeStep();
  if (!saved) return 0;
  if (saved === "usage-intent") return Math.max(0, steps.indexOf("area"));
  if (saved === "personality" || saved === "mood") return steps.indexOf("permissions");
  if (saved === "dictation-practice") return Math.max(0, steps.indexOf("permissions"));
  const index = steps.indexOf(saved as StepId);
  return index === -1 ? 0 : index;
}

function browserOnboardingDemoStep(): StepId | null {
  if (!import.meta.env.DEV || typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const step = params.get("clovyDemoStep") ?? params.get("juneDemoStep");
  if (step === "mood") return "permissions";
  return step === "area" || step === "permissions" ? step : null;
}

export function OnboardingFlow({ onComplete }: Props) {
  const t = useT();
  const capabilities = fallbackDictationCapabilities();
  const steps = ONBOARDING_STEPS;
  const [stepIndex, setStepIndex] = useState(() => initialStepIndex(steps));
  const [area, setArea] = useState<OnboardingArea>(() => onboardingArea() ?? "work");

  const stepId = steps[stepIndex];

  useEffect(() => {
    setOnboardingResumeStep(stepId);
  }, [stepId]);

  // Only poll the helper while the user is on the permissions screen.
  const permissionStatuses = usePermissionStatuses(stepId === "permissions");
  // The probe behind this is also what fires the system-audio TCC prompt
  // on a fresh install — deliberately run from the permissions screen, in
  // context, instead of ambushing the user after onboarding.
  const systemAudio = useSystemAudioStatus(stepId === "permissions");

  // Keep the existing fresh-install dictation default even though hands-on
  // practice happens after onboarding. A replay still must not clobber a
  // shortcut the user customized in Settings.
  useEffect(() => {
    if (capabilities.platform === "macos") {
      dictationSettings()
        .then(({ settings }) => {
          const current = settings.pushToTalkShortcut;
          if (current && !isFactoryDefaultShortcut(current)) return undefined;
          return setDictationShortcut("push_to_talk", FN_SHORTCUT);
        })
        .catch(() => undefined);
    }
  }, [capabilities.platform]);

  function goNext() {
    if (stepIndex >= steps.length - 1) {
      completeOnboarding();
      return;
    }
    setStepIndex((index) => Math.min(index + 1, steps.length - 1));
  }

  function completeOnboarding() {
    onComplete();
  }

  function goBack() {
    setStepIndex((index) => Math.max(index - 1, 0));
  }

  return (
    <div className="onboarding-screen">
      <header className="onboarding-topbar">
        {stepIndex > 0 ? (
          <button
            type="button"
            className="onboarding-back"
            onClick={goBack}
            aria-label={t("common.back")}
            title={t("common.back")}
          >
            <IconChevronLeftSmall size={18} aria-hidden />
          </button>
        ) : null}
        <nav
          className="onboarding-progress"
          aria-label={t("onboarding.progress", { step: stepIndex + 1, total: steps.length })}
        >
          {steps.map((id, index) => (
            <span
              key={id}
              className="onboarding-progress-seg"
              aria-hidden
              data-state={index < stepIndex ? "done" : index === stepIndex ? "current" : "upcoming"}
            />
          ))}
        </nav>
      </header>
      <div className="onboarding-body">
        {stepId === "area" ? (
          <AreaStep
            onContinue={(nextArea) => {
              setArea(nextArea);
              goNext();
            }}
          />
        ) : stepId === "permissions" ? (
          <PermissionsStep
            area={area}
            statuses={permissionStatuses}
            systemAudioStatus={systemAudio.status}
            onAllowSystemAudio={systemAudio.probe}
            onContinue={goNext}
          />
        ) : null}
      </div>
    </div>
  );
}
