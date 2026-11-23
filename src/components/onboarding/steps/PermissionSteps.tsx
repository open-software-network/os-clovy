import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconTextIndicator } from "central-icons/IconTextIndicator";
import { IconVolumeFull } from "central-icons/IconVolumeFull";
import { type MessageKey, useT } from "../../../i18n";
import type { OnboardingArea } from "../../../lib/onboarding";
import { fallbackDictationCapabilities } from "../../../lib/platform";
import { dictationHelperCommand, openPrivacySettings } from "../../../lib/tauri";
import { StepActions, StepCard } from "../StepChrome";
import {
  isAccessibilityGranted,
  isMicrophoneDenied,
  isMicrophoneGranted,
  type PermissionStatuses,
  type SystemAudioStatus,
} from "../use-permission-status";

const PERMISSION_COPY: Record<
  OnboardingArea,
  {
    subtitle: MessageKey;
    microphone: MessageKey;
    accessibility: MessageKey;
    systemAudio: MessageKey;
  }
> = {
  work: {
    subtitle: "onboarding.permissions.work.subtitle",
    microphone: "onboarding.permissions.work.microphone",
    accessibility: "onboarding.permissions.work.accessibility",
    systemAudio: "onboarding.permissions.work.systemAudio",
  },
  personal: {
    subtitle: "onboarding.permissions.personal.subtitle",
    microphone: "onboarding.permissions.personal.microphone",
    accessibility: "onboarding.permissions.personal.accessibility",
    systemAudio: "onboarding.permissions.captureOnRequest",
  },
  thinking: {
    subtitle: "onboarding.permissions.thinking.subtitle",
    microphone: "onboarding.permissions.thinking.microphone",
    accessibility: "onboarding.permissions.thinking.accessibility",
    systemAudio: "onboarding.permissions.captureOnRequest",
  },
  play: {
    subtitle: "onboarding.permissions.play.subtitle",
    microphone: "onboarding.permissions.play.microphone",
    accessibility: "onboarding.permissions.play.accessibility",
    systemAudio: "onboarding.permissions.captureOnRequest",
  },
};

function PermissionRow({
  icon,
  granted,
  probing = false,
  title,
  allowLabel,
  detail,
  onAllow,
}: {
  icon: ReactNode;
  granted: boolean;
  /** A permission check is in flight (the macOS dialog is up or about to
   * be); the row pulses so the wait reads as activity, not a stall. */
  probing?: boolean;
  title: string;
  /** Accessible name of the allow button ("Allow microphone access"). */
  allowLabel: string;
  detail: string;
  /** Grant affordance — fires the TCC prompt or opens System Settings;
   * either way the user's decision is "allow". */
  onAllow?: () => void;
}) {
  const t = useT();
  return (
    <li className="onboarding-perm" data-granted={granted} data-probing={probing}>
      <span className="onboarding-perm-icon" aria-hidden>
        {granted ? <IconCheckmark2Small size={15} /> : icon}
      </span>
      <div className="onboarding-perm-copy">
        <h2>{title}</h2>
        <p>{detail}</p>
      </div>
      {!granted && onAllow ? (
        <button
          type="button"
          className="onboarding-perm-btn"
          onClick={onAllow}
          aria-label={allowLabel}
        >
          {t("onboarding.permissions.allow")}
        </button>
      ) : null}
    </li>
  );
}

export function PermissionsStep({
  area,
  statuses,
  systemAudioStatus,
  onAllowSystemAudio,
  onContinue,
}: {
  area: OnboardingArea;
  statuses: PermissionStatuses;
  systemAudioStatus: SystemAudioStatus;
  /** Re-runs the capture-helper probe; fires the TCC prompt while the
   * permission is still undetermined. */
  onAllowSystemAudio: () => void;
  onContinue: () => void;
}) {
  const t = useT();
  const [showUnknownStatuses, setShowUnknownStatuses] = useState(false);
  const micGranted = isMicrophoneGranted(statuses);
  const micDenied = isMicrophoneDenied(statuses);
  const micUnavailable = statuses.microphone === "unavailable";
  const accessibilityGranted = isAccessibilityGranted(statuses);
  const systemAudioGranted = systemAudioStatus === "granted";
  const systemAudioDenied = systemAudioStatus === "denied";
  // macOS < 14.2 (or a missing capture helper) can never grant; the row
  // explains itself and stays out of the Continue gate.
  const systemAudioUnsupported = systemAudioStatus === "unsupported";
  // Granted, but the helper cannot capture until Clovy restarts. Onboarding has
  // nothing left to ask for, so the row explains itself and clears the gate
  // too. It is not marked granted: the source does not work yet.
  const systemAudioUnavailable = systemAudioStatus === "unavailable";
  const systemAudioSettled = systemAudioGranted || systemAudioUnsupported || systemAudioUnavailable;
  const showPermissionRows = statuses.checked || showUnknownStatuses;
  const capabilities = fallbackDictationCapabilities();
  const macLikePlatform = capabilities.platform === "macos";
  const windowsPlatform = capabilities.platform === "windows";
  const copy = PERMISSION_COPY[area];

  // Fire the native TCC prompt as soon as the screen shows — the user just
  // read why we're asking, so the dialog lands in context. No-op when
  // already granted; for already-denied users the helper emits the current
  // status so the System Settings fallback renders instead.
  useEffect(() => {
    void dictationHelperCommand({
      type: "request_microphone_permission",
    }).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (statuses.checked) {
      setShowUnknownStatuses(false);
      return;
    }

    const timer = window.setTimeout(() => {
      setShowUnknownStatuses(true);
    }, 240);

    return () => window.clearTimeout(timer);
  }, [statuses.checked]);

  function openAccessibilitySettings() {
    // Fire the helper's prompting check first: it registers the dictation
    // helper in the Accessibility list (so there's a toggle to flip) and
    // shows the native dialog. Let macOS own the System Settings handoff so
    // the native prompt is not left open behind a programmatic settings launch.
    void dictationHelperCommand({
      type: "request_accessibility_permission",
    }).catch(() => undefined);
  }

  return (
    <StepCard
      title={t("onboarding.permissions.title")}
      subtitle={
        macLikePlatform
          ? t(copy.subtitle)
          : windowsPlatform
            ? t("onboarding.permissions.subtitle.windows")
            : t("onboarding.permissions.subtitle.other")
      }
      wide
    >
      <ul
        className="onboarding-perms"
        data-checking={!showPermissionRows}
        aria-busy={!showPermissionRows}
      >
        <PermissionRow
          icon={<IconMicrophone size={15} />}
          granted={showPermissionRows && micGranted}
          title={t("onboarding.permissions.microphone.title")}
          allowLabel={t("onboarding.permissions.microphone.allowAria")}
          detail={
            micUnavailable
              ? t("onboarding.permissions.microphone.unavailable")
              : micDenied
                ? macLikePlatform
                  ? t("onboarding.permissions.microphone.deniedMac")
                  : t("onboarding.permissions.microphone.deniedWindows")
                : t(copy.microphone)
          }
          onAllow={
            showPermissionRows
              ? micUnavailable
                ? () =>
                    void dictationHelperCommand({
                      type: "get_permission_status",
                    }).catch(() => undefined)
                : micDenied
                  ? () => void openPrivacySettings("microphone")
                  : () =>
                      void dictationHelperCommand({
                        type: "request_microphone_permission",
                      }).catch(() => undefined)
              : undefined
          }
        />
        {macLikePlatform ? (
          <>
            <PermissionRow
              icon={<IconTextIndicator size={15} />}
              granted={showPermissionRows && accessibilityGranted}
              title={t("onboarding.permissions.accessibility.title")}
              allowLabel={t("onboarding.permissions.accessibility.allowAria")}
              detail={t(copy.accessibility)}
              onAllow={showPermissionRows ? openAccessibilitySettings : undefined}
            />
            <PermissionRow
              icon={<IconVolumeFull size={15} />}
              granted={showPermissionRows && systemAudioGranted}
              probing={showPermissionRows && systemAudioStatus === "probing"}
              title={t("onboarding.permissions.systemAudio.title")}
              allowLabel={t("onboarding.permissions.systemAudio.allowAria")}
              detail={
                systemAudioDenied
                  ? t("onboarding.permissions.systemAudio.denied")
                  : systemAudioUnsupported
                    ? t("onboarding.permissions.systemAudio.unsupported")
                    : systemAudioUnavailable
                      ? t("onboarding.permissions.systemAudio.unavailable")
                      : systemAudioStatus === "probing"
                        ? t("onboarding.permissions.systemAudio.probing")
                        : t(copy.systemAudio)
              }
              onAllow={
                showPermissionRows
                  ? systemAudioDenied
                    ? () => void openPrivacySettings("systemAudio")
                    : systemAudioStatus === "unknown"
                      ? onAllowSystemAudio
                      : undefined
                  : undefined
              }
            />
          </>
        ) : null}
      </ul>
      <StepActions
        onContinue={onContinue}
        continueDisabled={
          !showPermissionRows ||
          !micGranted ||
          (macLikePlatform && (!accessibilityGranted || !systemAudioSettled))
        }
        onSkip={onContinue}
      />
    </StepCard>
  );
}
