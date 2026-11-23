import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OnboardingFlow } from "../components/onboarding/OnboardingFlow";
import {
  applyOnboardingReplayFlag,
  isAgentRiskAcknowledged,
  isOnboardingComplete,
  markOnboardingComplete,
  ONBOARDING_COMPLETED_EVENT,
  onboardingArea,
  onboardingResumeStep,
  resetOnboardingForReplay,
  setOnboardingResumeStep,
  subscribeToOnboardingComplete,
} from "../lib/onboarding";
import type { RecordingSourceReadinessDto } from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  dictationSettings: vi.fn(),
  dictationHelperCommand: vi.fn(),
  checkRecordingSourceReadiness: vi.fn(),
  openPrivacySettings: vi.fn(),
  setDictationLanguage: vi.fn(),
  setDictationShortcut: vi.fn(),
  setClovyPersona: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  dictationCapabilities: vi.fn().mockResolvedValue({
    capabilities: {
      available: true,
      platform: "macos",
      shortcuts: true,
      paste: true,
      microphoneSelection: true,
      accessibilityPermission: true,
      systemAudio: true,
    },
  }),
  dictationSettings: mocks.dictationSettings,
  dictationHelperCommand: mocks.dictationHelperCommand,
  checkRecordingSourceReadiness: mocks.checkRecordingSourceReadiness,
  openPrivacySettings: mocks.openPrivacySettings,
  setDictationLanguage: mocks.setDictationLanguage,
  setDictationShortcut: mocks.setDictationShortcut,
  setClovyPersona: mocks.setClovyPersona,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

type ListenHandler = (event: { payload: string }) => void;

// What check_recording_source_readiness returns after the capture-helper
// probe: a passing probe reports the system source as granted; a denial
// flips both ready and permissionState.
function systemAudioReadiness(granted: boolean): RecordingSourceReadinessDto {
  return {
    sourceMode: "microphonePlusSystem",
    ready: granted,
    sources: [
      {
        source: "microphone",
        required: true,
        ready: true,
        permissionState: "granted",
        deviceAvailable: true,
        captureAvailable: true,
      },
      {
        source: "system",
        required: true,
        ready: granted,
        permissionState: granted ? "granted" : "denied",
        deviceAvailable: granted,
        captureAvailable: granted,
        recoveryAction: "openSystemAudioSettings",
      },
    ],
  };
}

function systemAudioCaptureUnavailableReadiness(): RecordingSourceReadinessDto {
  const readiness = systemAudioReadiness(false);
  const system = readiness.sources.find((source) => source.source === "system");
  if (system) {
    system.permissionState = "granted";
    system.deviceAvailable = true;
    system.captureAvailable = false;
    system.recoveryAction = "restartApp";
    system.message = "Failed to create audio format for system tap.";
  }
  return readiness;
}

function shortcut(label: string) {
  return {
    code: "Fn",
    label,
    pressCount: 1 as const,
    modifiers: {
      command: false,
      control: false,
      option: false,
      shift: false,
      function: true,
    },
  };
}

describe("OnboardingFlow", () => {
  let emitDictationEvent: ListenHandler | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    window.history.replaceState({}, "", "/");
    emitDictationEvent = undefined;
    mocks.listen.mockImplementation((eventName: string, handler: ListenHandler) => {
      if (eventName === "dictation-event") emitDictationEvent = handler;
      return Promise.resolve(vi.fn());
    });
    mocks.dictationHelperCommand.mockResolvedValue(undefined);
    mocks.checkRecordingSourceReadiness.mockResolvedValue(systemAudioReadiness(true));
    mocks.openPrivacySettings.mockResolvedValue(undefined);
    mocks.setDictationLanguage.mockResolvedValue(undefined);
    mocks.setDictationShortcut.mockResolvedValue(undefined);
    mocks.setClovyPersona.mockResolvedValue(undefined);
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: shortcut("fn"),
        toggleShortcut: shortcut("fn fn"),
        microphone: {},
        style: "standard",
        language: undefined,
      },
    });
  });

  function flowProps(overrides: Partial<Parameters<typeof OnboardingFlow>[0]> = {}) {
    return {
      onComplete: vi.fn(),
      ...overrides,
    };
  }

  async function renderFlow(onComplete = vi.fn()) {
    render(<OnboardingFlow {...flowProps({ onComplete })} />);
    await screen.findByRole("heading", { name: "Where could I help most?" });
    await userEvent.click(screen.getByRole("radio", { name: /Work/ }));
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByRole("heading", { name: "Let Clovy listen and type" });
    return onComplete;
  }

  function grantPermissions() {
    emitDictationEvent?.({
      payload: JSON.stringify({
        type: "permission_status",
        payload: { microphone: "granted", accessibility: "granted" },
      }),
    });
  }

  it("prefers the Clovy demo-step query parameter", async () => {
    window.history.replaceState({}, "", "/?clovyDemoStep=area&juneDemoStep=permissions");

    render(<OnboardingFlow {...flowProps()} />);

    expect(await screen.findByRole("heading", { name: "Where could I help most?" })).toBeVisible();
  });

  it("resumes a legacy personality step at permissions", async () => {
    setOnboardingResumeStep("mood");
    render(<OnboardingFlow {...flowProps()} />);
    expect(await screen.findByRole("heading", { name: "Let Clovy listen and type" })).toBeVisible();
  });

  it("accepts the June-era demo-step query parameter as a fallback", async () => {
    window.history.replaceState({}, "", "/?juneDemoStep=permissions");

    render(<OnboardingFlow {...flowProps()} />);

    expect(await screen.findByRole("heading", { name: "Let Clovy listen and type" })).toBeVisible();
  });

  function stubNavigatorPlatform(platform: string, userAgent: string) {
    const ownPlatform = Object.getOwnPropertyDescriptor(navigator, "platform");
    const ownUserAgent = Object.getOwnPropertyDescriptor(navigator, "userAgent");
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      get: () => platform,
    });
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      get: () => userAgent,
    });
    return () => {
      if (ownPlatform) {
        Object.defineProperty(navigator, "platform", ownPlatform);
      } else {
        Reflect.deleteProperty(navigator, "platform");
      }
      if (ownUserAgent) {
        Object.defineProperty(navigator, "userAgent", ownUserAgent);
      } else {
        Reflect.deleteProperty(navigator, "userAgent");
      }
    };
  }

  function stubMacNavigatorPlatform() {
    return stubNavigatorPlatform("MacIntel", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)");
  }

  it("walks the full flow", async () => {
    const user = userEvent.setup();
    const onComplete = await renderFlow();

    // Permissions: continue stays locked until the helper reports both granted.
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    grantPermissions();
    await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledOnce());

    expect(onboardingArea()).toBe("work");
    // Completion is the caller's job (App marks it), not the flow's.
    expect(isOnboardingComplete()).toBe(false);
  });

  it("asks where Clovy should help before requesting permissions", async () => {
    const user = userEvent.setup();
    setOnboardingResumeStep("area");
    render(<OnboardingFlow {...flowProps()} />);

    await screen.findByRole("heading", { name: "Where could I help most?" });
    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(continueButton).toBeDisabled();

    await user.click(screen.getByRole("radio", { name: /Thinking/ }));
    expect(continueButton).toBeEnabled();
    await user.click(continueButton);

    expect(onboardingArea()).toBe("thinking");
    await screen.findByRole("heading", { name: "Let Clovy listen and type" });
  });

  it("normalizes the factory-default shortcut to fn", async () => {
    // A fresh install still carries the Rust-side Ctrl+Opt+D default; only
    // then does onboarding write the bare-fn product default.
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          keyCode: 0x02,
          code: "KeyD",
          label: "Ctrl+Opt+D",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        toggleShortcut: shortcut("fn fn"),
        microphone: {},
        style: "standard",
        language: undefined,
      },
    });
    render(<OnboardingFlow {...flowProps()} />);
    await screen.findByRole("heading", { name: "Where could I help most?" });

    await waitFor(() =>
      expect(mocks.setDictationShortcut).toHaveBeenCalledWith(
        "push_to_talk",
        expect.objectContaining({ code: "Fn" }),
      ),
    );
  });

  it("keeps a customized shortcut on a wizard replay", async () => {
    // A version bump replays the wizard for existing users; a key they set
    // in Settings must survive untouched.
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          keyCode: 0x60,
          code: "F5",
          label: "F5",
          pressCount: 1,
          modifiers: {
            command: false,
            control: false,
            option: false,
            shift: false,
            function: false,
          },
        },
        toggleShortcut: shortcut("fn fn"),
        microphone: {},
        style: "standard",
        language: undefined,
      },
    });
    render(<OnboardingFlow {...flowProps()} />);
    await screen.findByRole("heading", { name: "Where could I help most?" });

    await waitFor(() => expect(mocks.dictationSettings).toHaveBeenCalledOnce());
    expect(mocks.setDictationShortcut).not.toHaveBeenCalled();
  });

  it("resumes a legacy practice step at permissions", async () => {
    setOnboardingResumeStep("dictation-practice");
    render(<OnboardingFlow {...flowProps()} />);
    await screen.findByRole("heading", { name: "Let Clovy listen and type" });
    expect(onboardingResumeStep()).toBe("permissions");
  });

  it("resets only onboarding progress when replaying the wizard", () => {
    markOnboardingComplete();
    setOnboardingResumeStep("setup");
    localStorage.setItem("clovy.agent.riskAcknowledged", "true");

    resetOnboardingForReplay();

    expect(isOnboardingComplete()).toBe(false);
    expect(onboardingResumeStep()).toBeNull();
    expect(isAgentRiskAcknowledged()).toBe(true);
  });

  it("applies the replay flag only in development", () => {
    markOnboardingComplete();
    setOnboardingResumeStep("setup");

    applyOnboardingReplayFlag({
      DEV: false,
      VITE_CLOVY_REPLAY_ONBOARDING: "1",
    });

    expect(isOnboardingComplete()).toBe(true);
    expect(onboardingResumeStep()).toBe("setup");

    applyOnboardingReplayFlag({
      DEV: true,
      VITE_CLOVY_REPLAY_ONBOARDING: "1",
    });

    expect(isOnboardingComplete()).toBe(false);
    expect(onboardingResumeStep()).toBeNull();
  });

  it("requests the mic permission when the mic screen shows", async () => {
    await renderFlow();
    await waitFor(() =>
      expect(mocks.dictationHelperCommand).toHaveBeenCalledWith({
        type: "request_microphone_permission",
      }),
    );
  });

  it("shows no-device guidance on Windows without opening privacy settings", async () => {
    const restoreNavigator = stubNavigatorPlatform(
      "Win32",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    );
    try {
      await renderFlow();
      emitDictationEvent?.({
        payload: JSON.stringify({
          type: "permission_status",
          payload: {
            microphone: "unavailable",
            microphoneDeviceAvailable: false,
            microphoneReason: "no_input_device",
            accessibility: "granted",
          },
        }),
      });

      expect(
        await screen.findByText(
          "No microphone found. Connect one, choose it in Windows sound settings, then try again.",
        ),
      ).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

      await userEvent.click(screen.getByRole("button", { name: "Allow microphone access" }));
      expect(mocks.openPrivacySettings).not.toHaveBeenCalled();
      expect(mocks.dictationHelperCommand).toHaveBeenCalledWith({
        type: "get_permission_status",
      });
    } finally {
      restoreNavigator();
    }
  });

  it("only requires microphone access on Windows", async () => {
    const restoreNavigator = stubNavigatorPlatform(
      "Win32",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    );
    try {
      const onComplete = vi.fn();
      await renderFlow(onComplete);

      expect(
        screen.getByText("Dictation and meeting notes need microphone access."),
      ).toBeInTheDocument();
      expect(screen.queryByText("Accessibility")).not.toBeInTheDocument();
      expect(screen.queryByText("System audio")).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

      emitDictationEvent?.({
        payload: JSON.stringify({
          type: "permission_status",
          payload: { microphone: "granted", accessibility: "missing" },
        }),
      });

      await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
      await userEvent.click(screen.getByRole("button", { name: "Continue" }));

      await waitFor(() => expect(onComplete).toHaveBeenCalledOnce());
      expect(screen.queryByRole("heading", { name: "Talk to Clovy" })).not.toBeInTheDocument();
    } finally {
      restoreNavigator();
    }
  });

  it("probes system audio when the macOS permissions screen shows", async () => {
    // The probe is what surfaces the system-audio TCC prompt on a fresh
    // install; it must fire here, in context, not after onboarding.
    const restoreNavigator = stubMacNavigatorPlatform();
    try {
      await renderFlow();
      await waitFor(() =>
        expect(mocks.checkRecordingSourceReadiness).toHaveBeenCalledWith("microphonePlusSystem"),
      );
    } finally {
      restoreNavigator();
    }
  });

  it("keeps continue locked and falls back to settings when system audio is denied", async () => {
    const user = userEvent.setup();
    const restoreNavigator = stubMacNavigatorPlatform();
    mocks.checkRecordingSourceReadiness.mockResolvedValue(systemAudioReadiness(false));
    try {
      await renderFlow();
      grantPermissions();

      await screen.findByText(
        "Turned off in System Settings. Flip the toggle and Clovy will notice.",
      );
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

      await user.click(screen.getByRole("button", { name: "Allow system audio access" }));
      expect(mocks.openPrivacySettings).toHaveBeenCalledWith("systemAudio");

      // The user flips the toggle and comes back; the focus re-probe picks
      // up the grant.
      mocks.checkRecordingSourceReadiness.mockResolvedValue(systemAudioReadiness(true));
      window.dispatchEvent(new Event("focus"));
      await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
    } finally {
      restoreNavigator();
    }
  });

  it("does not block continue when system audio is unsupported", async () => {
    const restoreNavigator = stubMacNavigatorPlatform();
    const readiness = systemAudioReadiness(false);
    const sysIdx = readiness.sources.findIndex((s) => s.source === "system");
    readiness.sources[sysIdx] = {
      ...readiness.sources[sysIdx],
      permissionState: "unsupported",
    };
    mocks.checkRecordingSourceReadiness.mockResolvedValue(readiness);
    try {
      await renderFlow();
      grantPermissions();

      await screen.findByText("Needs macOS 14.2 or later.");
      await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
    } finally {
      restoreNavigator();
    }
  });

  it("does not show System Settings copy when system audio permission is granted but capture is unavailable", async () => {
    const restoreNavigator = stubMacNavigatorPlatform();
    mocks.checkRecordingSourceReadiness.mockResolvedValue(systemAudioCaptureUnavailableReadiness());
    try {
      await renderFlow();
      grantPermissions();

      expect(
        screen.queryByText("Turned off in System Settings. Flip the toggle and Clovy will notice."),
      ).not.toBeInTheDocument();
      await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
    } finally {
      restoreNavigator();
    }
  });

  it("says system audio needs a restart rather than calling it allowed", async () => {
    const restoreNavigator = stubMacNavigatorPlatform();
    mocks.checkRecordingSourceReadiness.mockResolvedValue(systemAudioCaptureUnavailableReadiness());
    try {
      await renderFlow();
      grantPermissions();

      // The grant exists, so there is nothing left to allow, but the source
      // does not work yet and the row must not claim otherwise.
      await screen.findByText("Allowed. Restart Clovy to finish turning it on.");
      expect(
        screen.queryByText("Hears your calls and meetings, only while you record."),
      ).not.toBeInTheDocument();
      expect(mocks.openPrivacySettings).not.toHaveBeenCalled();
    } finally {
      restoreNavigator();
    }
  });
});

describe("subscribeToOnboardingComplete", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("fires the callback at most once even when both signals arrive", () => {
    const callback = vi.fn();
    const unsubscribe = subscribeToOnboardingComplete(callback);

    // A sibling window (the HUD) receives both the storage event and the
    // BroadcastChannel message for the same completion; the guard collapses
    // them into a single invocation.
    localStorage.setItem("clovy.onboarding.completedVersion", "999");
    window.dispatchEvent(new StorageEvent("storage", { key: "clovy.onboarding.completedVersion" }));
    window.dispatchEvent(new Event(ONBOARDING_COMPLETED_EVENT));

    expect(callback).toHaveBeenCalledOnce();
    unsubscribe();
  });

  it("never fires after unsubscribe", () => {
    const callback = vi.fn();
    const unsubscribe = subscribeToOnboardingComplete(callback);
    unsubscribe();

    localStorage.setItem("clovy.onboarding.completedVersion", "999");
    window.dispatchEvent(new StorageEvent("storage", { key: "clovy.onboarding.completedVersion" }));
    window.dispatchEvent(new Event(ONBOARDING_COMPLETED_EVENT));

    expect(callback).not.toHaveBeenCalled();
  });
});
