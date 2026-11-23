export type AgentChatTextPart = { type: "text"; text: string; status?: "running" | "complete" };
export type AgentChatReasoningPart = {
  type: "reasoning";
  text: string;
  status: "running" | "complete";
};
export type AgentChatContextPart = {
  type: "context";
  text: string;
  preview: string;
  status: "complete";
};
export type AgentChatToolPart = {
  type: "tool";
  id: string;
  name: string;
  text: string;
  status: "running" | "complete" | "failed" | "interrupted";
};
export type AgentApprovalChoice = "once" | "session" | "always" | "deny";
export type AgentChatApprovalPart = {
  type: "approval";
  id: string;
  runId: string;
  sessionId?: string;
  command: string;
  description: string;
  allowPermanent: boolean;
  choice?: AgentApprovalChoice;
  status: "pending" | "resolved" | "expired" | "cancelled";
  retiredReason?: string;
  /** Asked by a CLI turn through the approval broker. */
  engine?: "cli";
};
export type AgentChatClarifyPart = {
  type: "clarify";
  id: string;
  runId: string;
  sessionId?: string;
  question: string;
  choices: string[];
  answer?: string;
  status: "pending" | "resolved" | "expired" | "cancelled";
  engine?: "cli";
};
export type AgentChatSudoPart = {
  type: "sudo";
  id: string;
  command?: string;
  reason?: string;
  mode?: "sandboxed" | "unrestricted";
  approved?: boolean;
  status: "pending" | "resolved";
};

export const UPSTREAM_PROVIDER_FAILURE_NOTICE_BODY =
  "The upstream provider could not complete this request.";
export type AgentChatSecretPart = {
  type: "secret";
  id: string;
  runId: string;
  keyName?: string;
  reason?: string;
  status: "pending" | "resolved";
};
export type AgentChatNoticePart = {
  type: "notice";
  kind: "context-overflow" | "upstream-provider" | "tool" | "runtime";
  text: string;
  /** Sanitized workspace recovery observation; never a model or tool result. */
  diagnostic?: string;
  retryable?: boolean;
};
export type AgentChatSteeringPart = { type: "steering"; text: string };
export type AgentChatAttachmentPart = {
  type: "attachment";
  name: string;
  path: string;
  kind: "image" | "file";
};
export type AgentChatImagePart = {
  type: "image";
  status: "running" | "complete" | "error";
  prompt: string;
  requestId?: string;
  model?: string;
  safeMode?: boolean;
  userCreatedAt?: string;
  imageCreatedAt?: string;
  path?: string;
  dataUrl?: string;
  name?: string;
  error?: string;
};
export type AgentChatVideoPart = {
  type: "video";
  status: "running" | "complete" | "error";
  prompt: string;
  requestId?: string;
  model?: string;
  userCreatedAt?: string;
  videoCreatedAt?: string;
  jobId?: string;
  path?: string;
  posterDataUrl?: string;
  name?: string;
  error?: string;
};

export type AgentChatPart =
  | AgentChatTextPart
  | AgentChatReasoningPart
  | AgentChatContextPart
  | AgentChatToolPart
  | AgentChatApprovalPart
  | AgentChatClarifyPart
  | AgentChatSudoPart
  | AgentChatSecretPart
  | AgentChatNoticePart
  | AgentChatSteeringPart
  | AgentChatAttachmentPart
  | AgentChatImagePart
  | AgentChatVideoPart;

export type AgentChatTurn = {
  id: string;
  branchMessageId?: string;
  role: "user" | "assistant" | "system";
  createdAt: string;
  status: "running" | "complete";
  parts: AgentChatPart[];
  isScheduledRun?: boolean;
};

const CONTRACTION_GLUE = /([A-Za-z])('(?:s|re|ve|ll|m|d|t))(?=[A-Za-z])/gi;

export function repairContractionSpacing(text: string): string {
  return text.replace(CONTRACTION_GLUE, (whole, pre: string, enclitic: string) =>
    pre.toLowerCase() === "s" ? whole : `${pre}${enclitic} `,
  );
}

const MEDIA_REFERENCE =
  /MEDIA:(?:\/[^\r\n]+?|[A-Za-z0-9._-]+?)\.(?:png|jpe?g|gif|webp|tiff?|bmp|avif|mp4|mov|webm|m4v)(?:[)\].,;:]?)(?=\s|$)/gi;

export function stripRenderedMediaReferences(value: string, holdTrailingPartial = false): string {
  const stripped = value
    .replace(MEDIA_REFERENCE, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n");
  return holdTrailingPartial
    ? stripped.replace(/(^|\r?\n)[ \t]*(?:M|ME|MED|MEDI|MEDIA|MEDIA:.*)$/i, "$1")
    : stripped;
}

export function displayedComposerUserMessageText(content: string): string {
  return content
    .replace(/\n*\[Image attached at:\s*[^\]]+\]\s*(?:\[[^\]\n]+\]\s*)*$/i, "")
    .replace(/^\s*\[(?:Clovy|June) attachment manifest v1\][\s\S]*?\n\n/, "")
    .trim();
}
