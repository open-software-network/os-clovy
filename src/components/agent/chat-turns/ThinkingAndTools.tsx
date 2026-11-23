import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconConsoleSimple } from "central-icons/IconConsoleSimple";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import type { ReactNode } from "react";
import { useT } from "../../../i18n";
import type { AgentChatPart } from "../../../lib/agent-chat-runtime";
import { DotSpinner } from "../../DotSpinner";

export function AgentThinkingGroup({
  open,
  onOpenChange,
  reasoning,
  running,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  reasoning: Extract<AgentChatPart, { type: "reasoning" }>[];
  running: boolean;
}) {
  const t = useT();
  const reduceMotion = useReducedMotion();
  // Collapsed by default to a short label — "Thinking" while it works, "Thought"
  // once done (terracotta while live). Expanding reveals only the reasoning
  // prose; tool/action rows render outside this disclosure.
  const reasoningText = reasoning
    .map((part) => part.text)
    .join("\n\n")
    .trim();
  return (
    <details
      className="agent-reasoning"
      data-status={running ? "running" : "completed"}
      open={open}
      onToggle={(event) => onOpenChange(event.currentTarget.open)}
    >
      <summary aria-label={running ? t("chat.thinking.running") : t("chat.thinking.done")}>
        <span className="agent-reasoning-label-swap" aria-hidden="true">
          <AnimatePresence initial={false}>
            <motion.span
              key={running ? "thinking" : "thought"}
              className={running ? "text-shimmer shimmer" : undefined}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{
                // Framer Motion takes seconds; these mirror --t-fast/--t-med.
                duration: reduceMotion ? 0.1 : 0.16,
                ease: [0.22, 1, 0.36, 1],
              }}
            >
              {running ? t("chat.thinking.running") : t("chat.thinking.done")}
            </motion.span>
          </AnimatePresence>
        </span>
        <IconChevronDownSmall size={14} className="agent-disclosure-chevron" />
      </summary>
      <div className="agent-reasoning-body">
        {reasoningText ? <div className="agent-reasoning-text">{reasoningText}</div> : null}
      </div>
    </details>
  );
}

// Tool activity is collapsed to a single quiet row by default — name + status —
// so the conversation isn't buried under raw tool output (skill dumps, command
// logs). The full output is one click away when the row has a body.
function AgentToolDisclosure({
  name,
  status,
  statusNode,
  text,
  redacted,
}: {
  name: string;
  status: string;
  statusNode: ReactNode;
  text?: string | null;
  redacted?: boolean;
}) {
  const t = useT();
  const body = text && text.trim() ? text : null;
  const summary = (expandable: boolean) => (
    <>
      {/* On hover the tool glyph cross-fades to a plain-text affordance —
       * "+" when closed, "−" when open. Text instead of svg icons: glyphs
       * render on the text baseline grid, so the swap can't hitch a pixel. */}
      <span className="agent-tool-icon">
        <IconConsoleSimple size={15} className="agent-tool-icon-glyph" />
        {expandable ? (
          <>
            <span className="agent-tool-icon-expand">+</span>
            <span className="agent-tool-icon-minimize">−</span>
          </>
        ) : null}
      </span>
      <span className="agent-tool-name">{name}</span>
      {statusNode}
      {redacted ? <span className="agent-redacted">{t("chat.tools.redacted")}</span> : null}
    </>
  );
  if (!body) {
    return (
      <div className="agent-tool-disclosure agent-tool-disclosure-static" data-status={status}>
        {summary(false)}
      </div>
    );
  }
  return (
    <details className="agent-tool-disclosure" data-status={status}>
      <summary>{summary(true)}</summary>
      <div className="agent-tool-output">{body}</div>
    </details>
  );
}

function AgentToolPartRow({ part }: { part: Extract<AgentChatPart, { type: "tool" }> }) {
  const t = useT();
  return (
    <AgentToolDisclosure
      name={part.name}
      status={part.status}
      text={part.text}
      statusNode={
        part.status === "running" ? (
          <span
            className="agent-tool-spinner"
            role="status"
            aria-label={t("chat.tools.running")}
            title={t("chat.tools.running")}
          >
            <DotSpinner />
          </span>
        ) : part.status === "failed" ? (
          <span className="agent-tool-live-status" data-status="failed">
            {t("chat.tools.failed")}
          </span>
        ) : part.status === "interrupted" ? (
          <span className="agent-tool-live-status" data-status="interrupted">
            {t("chat.tools.interrupted")}
          </span>
        ) : null
      }
    />
  );
}

// Long tool runs stop growing the transcript a row per call: past this many
// rows, settled calls fold behind a single count line while
// running calls stay visible below it, so what Clovy is doing right now is
// never hidden and failures are still called out on the fold itself.
const AGENT_TOOL_STACK_FOLD_THRESHOLD = 3;

export function AgentToolStack({ parts }: { parts: Extract<AgentChatPart, { type: "tool" }>[] }) {
  const t = useT();
  const settled = parts.filter((part) => part.status !== "running");
  const folded = parts.length > AGENT_TOOL_STACK_FOLD_THRESHOLD && settled.length >= 2;
  if (!folded) {
    return (
      <div className="agent-tool-stack">
        {parts.map((tool) => (
          <AgentToolPartRow key={`tool:${tool.id}`} part={tool} />
        ))}
      </div>
    );
  }
  const running = parts.filter((part) => part.status === "running");
  const failedCount = settled.filter((part) => part.status === "failed").length;
  const interruptedCount = settled.filter((part) => part.status === "interrupted").length;
  return (
    <div className="agent-tool-stack">
      {/* Uncontrolled like the per-row disclosures: the browser owns the open
       * state, so rows settling into the fold don't snap it shut. */}
      <details
        className="agent-tool-disclosure agent-tool-fold"
        data-status={failedCount > 0 ? "failed" : interruptedCount > 0 ? "interrupted" : "complete"}
      >
        <summary>
          <span className="agent-tool-icon">
            <IconConsoleSimple size={15} className="agent-tool-icon-glyph" />
            <span className="agent-tool-icon-expand">+</span>
            <span className="agent-tool-icon-minimize">−</span>
          </span>
          <span className="agent-tool-name">
            {t("chat.tools.actions", { count: settled.length })}
          </span>
          {failedCount > 0 ? (
            <span className="agent-tool-live-status" data-status="failed">
              {t("chat.tools.failedCount", { count: failedCount })}
            </span>
          ) : null}
          {interruptedCount > 0 ? (
            <span className="agent-tool-live-status" data-status="interrupted">
              {t("chat.tools.interruptedCount", { count: interruptedCount })}
            </span>
          ) : null}
        </summary>
        <div className="agent-tool-fold-body">
          {settled.map((tool) => (
            <AgentToolPartRow key={`tool:${tool.id}`} part={tool} />
          ))}
        </div>
      </details>
      {running.map((tool) => (
        <AgentToolPartRow key={`tool:${tool.id}`} part={tool} />
      ))}
    </div>
  );
}
