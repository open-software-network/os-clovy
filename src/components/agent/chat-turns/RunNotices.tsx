import { useT } from "../../../i18n";
import type { AgentChatPart } from "../../../lib/agent-chat-runtime";

export function ContextOverflowNoticePart() {
  const t = useT();
  return <div className="agent-system-notice">{t("chat.notice.contextOverflow")}</div>;
}

export function UpstreamProviderFailureNoticePart({
  onRetry,
  kind = "upstream-provider",
  diagnostic,
}: {
  onRetry?: () => void;
  kind?: "upstream-provider" | "tool" | "runtime";
  diagnostic?: string;
  [key: string]: unknown;
}) {
  const t = useT();
  const message = diagnostic
    ? t("chat.notice.workspaceConnectionRecovery")
    : kind === "tool"
      ? t("chat.notice.toolFailed")
      : kind === "runtime"
        ? t("chat.notice.runtimeStopped")
        : t("chat.notice.upstreamFailed");
  return (
    <div className="agent-system-notice">
      {message}
      {diagnostic ? <div>{diagnostic}</div> : null}
      {onRetry ? (
        <button type="button" onClick={onRetry}>
          {t("common.tryAgain")}
        </button>
      ) : null}
    </div>
  );
}

export function SteeringPart({ part }: { part: Extract<AgentChatPart, { type: "steering" }> }) {
  return <div className="agent-system-notice">{part.text}</div>;
}
