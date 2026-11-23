import type { SidebarView } from "../components/sidebar/Sidebar";
import type { RecordingStatusDto } from "../lib/tauri";
import type * as React from "react";

export type UseAgentAttentionNotificationsDependencies = {
  activeViewRef: React.MutableRefObject<SidebarView>;
  dictationWorkflowActiveRef: React.MutableRefObject<boolean>;
  recordingStatusRef: React.MutableRefObject<RecordingStatusDto | undefined>;
};
