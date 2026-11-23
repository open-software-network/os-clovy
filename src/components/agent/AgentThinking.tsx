import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useT } from "../../i18n";

// Framer transforms require same-shape numeric values, so this mirrors the
// 2px --sp-px token rather than passing that CSS variable into the mixer.
const AGENT_THINKING_OFFSET_PX = 2;

/**
 * Bottom-of-timeline responding affordance. The presence host stays mounted
 * while runtime events have no visible output, preserving the shimmer phase;
 * once output arrives, the label gets a brief handoff instead of disappearing.
 */
export function AgentThinking({
  visible,
  variant = "label",
}: {
  visible: boolean;
  variant?: "label" | "typing-bubble";
}) {
  const t = useT();
  const reduceMotion = useReducedMotion();

  return (
    <AnimatePresence initial={false} mode="popLayout">
      {visible ? (
        <motion.div
          key="agent-thinking"
          className="agent-thinking"
          data-variant={variant}
          role="status"
          aria-live="polite"
          aria-atomic="true"
          initial={reduceMotion ? { opacity: 0 } : { opacity: 0, y: AGENT_THINKING_OFFSET_PX }}
          animate={{ opacity: 1, y: 0 }}
          exit={reduceMotion ? { opacity: 0 } : { opacity: 0, y: -AGENT_THINKING_OFFSET_PX }}
          transition={{
            // Framer Motion takes seconds; these mirror --t-fast/--t-med. The
            // Home typing dots drift in slowly — ephemeral presence, not a
            // status flip.
            duration: reduceMotion ? 0.1 : variant === "typing-bubble" ? 0.55 : 0.16,
            ease: [0.22, 1, 0.36, 1],
          }}
        >
          {variant === "typing-bubble" ? (
            <>
              <span className="visually-hidden">{t("agent.thinking.typing")}</span>
              <span className="agent-typing-dots" aria-hidden="true">
                <span />
                <span />
                <span />
              </span>
            </>
          ) : (
            <span className="text-shimmer shimmer agent-thinking-label">
              {t("agent.thinking.label")}
            </span>
          )}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
