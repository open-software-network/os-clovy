import type { ReactNode } from "react";
import type { AgentProjectContext } from "../../lib/agent-project-context";
import type { AgentSessionDto } from "../../lib/agent-runtime-contract";
/** Where the session was opened from — rendered as the leading crumbs in the
 * sticky session bar ("Projects / Clovy" or "Agents") with a back arrow. */
export type AgentWorkspaceOrigin = {
  backLabel: string;
  onBack: () => void;
  crumbs: { label: string; icon?: ReactNode; onClick: () => void }[];
};

export type AgentWorkspaceProps = {
  initialSession?: AgentSessionDto;
  initialSessionId?: string;
  /** Persistent relationship-level conversation, distinct from focused sessions. */
  homeMode?: boolean;
  onHomeSessionCreated?: (storedSessionId: string) => void;
  onOpenHomeTaskSession?: (storedSessionId: string, title: string) => void;
  origin?: AgentWorkspaceOrigin;
  onSessionSelected?: (session: AgentSessionDto | undefined) => void;
  /** Whether the active session is filed in a project — drives the session
   * bar menu's project item label (App owns the folder state). */
  sessionInProject?: boolean;
  /** Current project metadata for hidden prompt context injection. */
  projectContext?: AgentProjectContext;
  /** Resolves the project a specific stored session is filed in. Background
   * deliveries (queued steers/attachments) target sessions other than the
   * active one; injecting the ambient `projectContext` there would leak the
   * open project's instructions into another session's run. */
  resolveSessionProjectContext?: (storedSessionId: string) => AgentProjectContext | undefined;
  /** Opens the change-project dialog (which also owns removal) for the given
   * stored session id. */
  onMoveSessionToProject?: (sessionId: string) => void;
  testOnlySlashCommandEntriesRef?: {
    current: {
      runImageSlashCommand: (argument: string, commandText: string) => Promise<void>;
      runVideoSlashCommand: (argument: string, commandText: string) => Promise<void>;
    } | null;
  };
};
