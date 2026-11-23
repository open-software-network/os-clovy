import type * as React from "react";

export type UseAppUpdateActionsDependencies = {
  // Onboarding owns the screen: no launch check until it is done.
  appBlocked: boolean;
  // A meeting recording is running (reactive).
  recordingActive: boolean;
  // A dictation take is running; tracked in a ref, so it is polled.
  dictationWorkflowActiveRef: React.MutableRefObject<boolean>;
  // Saves pending note edits before the restart.
  flushNotes: () => Promise<void>;
};
