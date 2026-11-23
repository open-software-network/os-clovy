import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconExclamationTriangle } from "central-icons/IconExclamationTriangle";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
import { useT } from "../../../i18n";
import type { AgentApprovalChoice } from "../../../lib/agent-chat-runtime";

export function ResolvedActionRow({
  denied = false,
  unknown = false,
  label,
  detail,
  children,
}: {
  /** Renders the cross glyph and destructive tint instead of the checkmark. */
  denied?: boolean;
  /** Renders a neutral warning glyph when the transport lost the outcome. */
  unknown?: boolean;
  /** Short outcome word(s), e.g. "Approved once" / "Answered" / "Denied". */
  label: string;
  /** One-line truncated detail shown inline on the collapsed row. */
  detail?: ReactNode;
  /** The full detail body revealed on expand. */
  children?: ReactNode;
}) {
  return (
    <details
      className="agent-tool-disclosure agent-resolved-row"
      data-choice={unknown ? "unknown" : denied ? "deny" : "done"}
    >
      <summary>
        <span className="agent-tool-icon">
          {unknown ? (
            <IconExclamationTriangle
              size={15}
              className="agent-tool-icon-glyph agent-resolved-icon-glyph"
            />
          ) : denied ? (
            <IconCrossSmall size={15} className="agent-tool-icon-glyph agent-resolved-icon-glyph" />
          ) : (
            <IconCheckmark2Small
              size={15}
              className="agent-tool-icon-glyph agent-resolved-icon-glyph"
            />
          )}
          <span className="agent-tool-icon-expand">+</span>
          <span className="agent-tool-icon-minimize">−</span>
        </span>
        <span className="agent-tool-name agent-resolved-label">{label}</span>
        {detail !== undefined ? <span className="agent-resolved-detail">{detail}</span> : null}
      </summary>
      {children !== undefined ? <div className="agent-resolved-body">{children}</div> : null}
    </details>
  );
}

/** The condensed chrome shared by the pending approval and sudo cards. The
 * header is a plain row (title + optional inline mode tag + waiting status) —
 * not a toggle. Below it the prose `description` reads at all times, clamped to
 * two lines while collapsed. When there is more to show (`hasDetails` — a
 * command, or the sudo mode notice) a quiet "Details" disclosure sits under the
 * description and reveals the full body (`children`: the full command `pre` and
 * any extra detail). The actions row (`footer`) is always visible. Collapsed by
 * default so a long command never dominates the card before a decision. */
export function CollapsibleActionCard({
  title,
  description,
  headerMeta,
  command,
  hasDetails,
  expanded,
  onToggleExpanded,
  footer,
  children,
}: {
  title: string;
  /** The prose description (part.description / sudo reason), always visible. */
  description: ReactNode;
  /** A short signal pinned to the header row that must stay visible while
   * collapsed (e.g. the sudo blast-radius mode tag). */
  headerMeta?: ReactNode;
  /** SECURITY: the concrete command being authorized. Rendered ALWAYS (never
   * behind the disclosure) so the exact command is visible at the decision
   * point — the Approve button is live while the card is collapsed, so a user
   * must be able to see what they are approving without expanding anything. */
  command?: ReactNode;
  /** Whether there is supplementary body content worth a "Details" disclosure
   * (e.g. the sudo mode notice). The command is NOT gated on this. */
  hasDetails: boolean;
  expanded: boolean;
  onToggleExpanded: () => void;
  /** The actions row (or the in-flight result line), always visible. */
  footer: ReactNode;
  /** Supplementary body revealed on expand (never the command). */
  children: ReactNode;
}) {
  const t = useT();
  return (
    <article
      className="agent-approval-card agent-action-card"
      data-status="pending"
      data-expanded={expanded || undefined}
    >
      <div className="agent-action-card-header">
        <span className="agent-action-card-title">{title}</span>
        {headerMeta}
      </div>
      {/* Only clamp when a Details expander exists to reveal the rest; otherwise
       * a long description-only request would be truncated with no way to read
       * it before choosing. */}
      <p
        className="agent-action-card-description"
        data-clamped={(hasDetails && !expanded) || undefined}
      >
        {description}
      </p>
      {command}
      {hasDetails ? (
        <button
          type="button"
          className="agent-action-card-details"
          aria-expanded={expanded}
          onClick={onToggleExpanded}
        >
          {t("chat.approval.details")}
          <IconChevronDownSmall size={14} className="agent-disclosure-chevron" aria-hidden />
        </button>
      ) : null}
      {expanded ? <div className="agent-action-card-body">{children}</div> : null}
      {footer}
    </article>
  );
}

/** The approval footer's primary control: a split button. "Approve" approves
 * "once"; the attached caret opens a small scope menu ("Approve once" /
 * "Always approve", the last hidden when `allowPermanent` is false).
 * Dismisses on outside click or Escape and supports
 * arrow-key navigation, mirroring the repo's other hand-rolled menus. */
export function ApproveSplitButton({
  disabled,
  allowPermanent,
  scopeMenu = true,
  onChoice,
}: {
  disabled: boolean;
  allowPermanent?: boolean;
  /** `false` drops the scope caret: the request can only be approved once
   * (approvals asked by a CLI turn grant nothing beyond the call). */
  scopeMenu?: boolean;
  onChoice: (choice: AgentApprovalChoice) => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const scopeRef = useRef<HTMLButtonElement | null>(null);

  // Close on a click outside the split wrapper or on Escape.
  useEffect(() => {
    if (!open) return;
    function onPointer(event: MouseEvent) {
      if (wrapRef.current?.contains(event.target as Node)) return;
      setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        // Escape is a keyboard dismissal — return focus to the caret trigger so
        // it doesn't drop to <body> when the focused menu item unmounts.
        scopeRef.current?.focus();
      }
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Move focus into the menu when it opens so arrow keys land immediately.
  useEffect(() => {
    if (!open) return;
    menuRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
  }, [open]);

  const items: { choice: AgentApprovalChoice; label: string }[] = [
    { choice: "once", label: t("chat.approval.approveOnce") },
    ...(allowPermanent
      ? [{ choice: "always" as AgentApprovalChoice, label: t("chat.approval.alwaysApprove") }]
      : []),
  ];

  function choose(choice: AgentApprovalChoice) {
    setOpen(false);
    onChoice(choice);
  }

  function onMenuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [],
    );
    if (!buttons.length) return;
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown") {
      event.preventDefault();
      buttons[(current + 1 + buttons.length) % buttons.length]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      buttons[(current - 1 + buttons.length) % buttons.length]?.focus();
    } else if (event.key === "Home") {
      event.preventDefault();
      buttons[0]?.focus();
    } else if (event.key === "End") {
      event.preventDefault();
      buttons[buttons.length - 1]?.focus();
    }
  }

  return (
    <div
      className={`agent-approval-split${disabled ? " agent-approval-split-disabled" : ""}`}
      ref={wrapRef}
    >
      <button
        type="button"
        className="agent-approval-approve"
        disabled={disabled}
        onClick={() => onChoice("once")}
      >
        {t("chat.approval.approve")}
      </button>
      {scopeMenu ? (
        <button
          ref={scopeRef}
          type="button"
          className="agent-approval-scope"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={t("chat.approval.approveOptions")}
          disabled={disabled}
          onClick={() => setOpen((value) => !value)}
        >
          <IconChevronDownSmall size={14} aria-hidden />
        </button>
      ) : null}
      {scopeMenu && open ? (
        <div
          ref={menuRef}
          className="agent-approval-scope-menu"
          role="menu"
          aria-label={t("chat.approval.approveScope")}
          onKeyDown={onMenuKeyDown}
        >
          {items.map((item) => (
            <button
              key={item.choice}
              type="button"
              role="menuitem"
              className="agent-approval-scope-item"
              disabled={disabled}
              onClick={() => choose(item.choice)}
            >
              {item.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
