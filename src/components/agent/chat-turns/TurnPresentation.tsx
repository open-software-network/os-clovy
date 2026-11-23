import { useEffect, useId, useState } from "react";
import { useT } from "../../../i18n";
import type { AgentChatPart, AgentChatTurn } from "../../../lib/agent-chat-runtime";

export const TURN_ACTION_TIP_DELAY_MS = 450;

export function turnIsConcreteResponse(turn: AgentChatTurn): boolean {
  return turn.parts.some((part) =>
    part.type === "text" ? Boolean(part.text.trim()) : part.type !== "reasoning",
  );
}

export function SudoPart({
  part,
}: {
  part: Extract<AgentChatPart, { type: "sudo" }>;
  [key: string]: unknown;
}) {
  const t = useT();
  return (
    <div className="agent-system-notice" data-status={part.status}>
      {part.reason || t("chat.approval.required")}
    </div>
  );
}

export function SecretPart({
  part,
  onSecret,
  submitting,
}: {
  part: Extract<AgentChatPart, { type: "secret" }>;
  onSecret: (part: Extract<AgentChatPart, { type: "secret" }>, value: string) => void;
  submitting?: true;
}) {
  const t = useT();
  const [value, setValue] = useState("");
  const inputId = useId();
  const disabled = part.status !== "pending" || submitting;

  useEffect(
    () => () => {
      setValue("");
    },
    [],
  );

  if (part.status === "resolved") {
    return <div className="agent-system-notice">{t("chat.secret.resolved")}</div>;
  }

  return (
    <form
      className="agent-action-card"
      data-status={part.status}
      onSubmit={(event) => {
        event.preventDefault();
        if (!value || disabled) return;
        onSecret(part, value);
        setValue("");
      }}
    >
      <div className="agent-action-card-body">
        <label htmlFor={inputId}>{t("chat.secret.required")}</label>
        <p>{part.reason || t("chat.secret.fallbackReason")}</p>
        <input
          id={inputId}
          className="dialog-input"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={value}
          disabled={disabled}
          onChange={(event) => setValue(event.currentTarget.value)}
        />
      </div>
      <div className="agent-approval-actions">
        <button type="submit" className="btn btn-secondary" disabled={!value || disabled}>
          {submitting ? t("chat.secret.submitting") : t("chat.secret.submit")}
        </button>
        <button
          type="button"
          className="btn btn-ghost agent-approval-deny"
          disabled={disabled}
          onClick={() => {
            setValue("");
            onSecret(part, "");
          }}
        >
          {t("common.cancel")}
        </button>
      </div>
    </form>
  );
}
