import React from "react";
import ReactDOM from "react-dom/client";
import { Agentation } from "agentation";
import { App } from "./app/App";
import { Toaster } from "./components/ui/Toaster";
import { installNativeContextMenuGuard } from "./lib/native-context-menu";
import { replayOnboarding } from "./lib/onboarding";
import { initInterfaceLocale } from "./i18n";
import { initTheme } from "./lib/theme";
import { initBrand } from "./lib/brand";
import { initFontScale, installFontScaleShortcuts } from "./lib/font-scale";
import { installExternalLinkOpener } from "./lib/external-links";
import { initializeExperimentalFlags } from "./lib/experimental-flags";
import {
  prefetchRemainingWorkspacesAfterPaint,
  preloadInitialWorkspace,
} from "./app/workspace-lazy";
import "./styles/app.css";

declare global {
  interface Window {
    /** Devtools-console testing hooks; not referenced by app code. */
    clovy?: { replayOnboarding: typeof replayOnboarding };
    /** June-era alias retained for existing development links and notes. */
    june?: { replayOnboarding: typeof replayOnboarding };
  }
}

// `clovy.replayOnboarding()` in the webview console re-runs the wizard;
// pass a step id ("permissions", "mood", ...) to land on that step.
if (import.meta.env.DEV) {
  const devtools = { replayOnboarding };
  window.clovy = devtools;
  window.june = devtools;
}

initInterfaceLocale();
initTheme();
initBrand();
initFontScale();
installFontScaleShortcuts();
installExternalLinkOpener();
installNativeContextMenuGuard();
// The Today route is allowed to depend on the entry chunk. Start loading it
// early without blocking the entry module on that dependency.
void preloadInitialWorkspace().catch(() => undefined);
await initializeExperimentalFlags();

if (import.meta.env.DEV) {
  void import("./lib/spinner-demo").then(({ registerSpinnerDemo }) => registerSpinnerDemo());
  // Same pattern for the meeting-detection prompt: __meetingHud("detected")
  // drives the real dictation HUD window over the Tauri bus.
  void import("./lib/meeting-hud-demo").then(({ registerMeetingHudDemo }) =>
    registerMeetingHudDemo({ local: false }),
  );
  // __dictationHud("listening") drives the dictation pill in the same HUD
  // window over the Tauri bus.
  void import("./lib/dictation-hud-demo").then(({ registerDictationHudDemo }) =>
    registerDictationHudDemo({ local: false }),
  );
  // __recordingHud("recording") drives the recording pill (meeting-hud window)
  // over the Tauri bus. Note: that window only shows when Rust already has a
  // live recording with the main window hidden — see lib/recording-hud-demo.ts.
  void import("./lib/recording-hud-demo").then(({ registerRecordingHudDemo }) =>
    registerRecordingHudDemo({ local: false }),
  );
  // __emptyStates() forces every list view (Routines, Projects,
  // Notes, Dictation, sidebar) into its empty rendering for design work;
  // call again or __emptyStates(false) to reset. Real data is untouched.
  void import("./lib/empty-states-demo").then(({ registerEmptyStatesDemo }) =>
    registerEmptyStatesDemo(),
  );
  // __projectMemoryDemo() drops sample memories into an open Project settings
  // dialog so the populated memory list can be designed; call again to reset.
  void import("./lib/project-memory-demo").then(({ registerProjectMemoryDemo }) =>
    registerProjectMemoryDemo(),
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
    {/* Global toast host. Sonner mounts a real in-flow <section> (no portal),
        so it must live outside App's .app-shell grid: an in-flow child there
        auto-places into an implicit second grid row and steals height from
        the main column once the sidebar collapses (JUN-237). The toast list
        itself is position: fixed, so it renders identically from here. */}
    <Toaster />
    {import.meta.env.DEV ? <Agentation /> : null}
  </React.StrictMode>,
);
prefetchRemainingWorkspacesAfterPaint();
