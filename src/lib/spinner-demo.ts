// Dev-only coordinator for inspecting Clovy's shared spinner in representative
// production contexts from one console command:
//
//   window.__spinnerDemo()       show sidebar, HUD, and toast samples
//   window.__spinnerDemo(false)  clear every sample
//
// The individual surfaces keep owning their own preview state. This module
// composes those existing drivers so it never starts a real request, mutates
// user data, or creates a second presentation-only version of a spinner.

import { toast } from "../components/ui/Toaster";
import { AGENT_HUD_VISIBILITY_CHANGED_EVENT, getAgentHudEnabled } from "./agent-hud-settings";

type SpinnerDemoWindow = Window & {
  __agentHud?: (state?: string, count?: number) => unknown;
  __sidebarStates?: (show?: boolean) => unknown;
  __spinnerDemo?: (show?: boolean) => string;
};

export type SpinnerDemoApi = {
  dispose: () => void;
};

function emitAgentHudVisibility(enabled: boolean) {
  void import("@tauri-apps/api/event")
    .then(({ emit }) => emit(AGENT_HUD_VISIBILITY_CHANGED_EVENT, { enabled }))
    .catch(() => {});
}

export function registerSpinnerDemo(): SpinnerDemoApi {
  const target = window as SpinnerDemoWindow;
  let active = false;
  let loadingToastId: string | number | undefined;

  function clear() {
    target.__agentHud?.("clear");
    target.__sidebarStates?.(false);
    if (loadingToastId !== undefined) {
      toast.dismiss(loadingToastId);
      loadingToastId = undefined;
    }
    // The demo may temporarily reveal a disabled HUD, but never changes the
    // saved preference. Teardown returns the Agent HUD to the current setting.
    emitAgentHudVisibility(getAgentHudEnabled());
    active = false;
  }

  const run = (show: boolean = true) => {
    if (!show) {
      clear();
      return "Spinner demo cleared.";
    }

    active = true;
    target.__sidebarStates?.(true);
    // "mixed" auto-expands the Agent HUD, exposing its running-row spinners.
    // A transient visibility event also reveals it when the saved preference
    // is off; clear() restores that preference without rewriting localStorage.
    emitAgentHudVisibility(true);
    target.__agentHud?.("mixed");
    loadingToastId ??= toast.loading("Spinner showcase");

    return "Spinner demo shown: sidebar, Agent HUD, and loading toast. Run __spinnerDemo(false) to clear.";
  };

  target.__spinnerDemo = run;

  return {
    dispose() {
      if (active) clear();
      if (target.__spinnerDemo === run) delete target.__spinnerDemo;
    },
  };
}
