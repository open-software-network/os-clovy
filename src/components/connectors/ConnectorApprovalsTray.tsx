import { IconChecklist } from "central-icons/IconChecklist";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { useCallback, useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import { actionToolLabel, providerFromServer } from "../../lib/connectors";
import { useConnectorPolicy } from "../../lib/connector-policy";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  CONNECTOR_APPROVALS_CHANGED_EVENT,
  type PendingConnectorApproval,
  connectorApprovalRespond,
  connectorApprovalsPending,
  connectorApprovalsRespondAll,
} from "../../lib/tauri";
import { ConnectorProviderIcon } from "./ConnectorProviderIcon";

/**
 * Always-mounted surface for connector action calls parked in the Rust proxy
 * under the approval trust mode. A routine (or chat) that tries to send, draft,
 * label, or change a calendar event pauses here until the user approves or
 * denies. Renders nothing when nothing is pending.
 *
 * Batched by design: a triage run can propose several drafts at once, so
 * "Approve all" answers the whole set in one tap. Every item shows the account
 * it touches and a redacted preview (redaction happens in Rust).
 */
export function ConnectorApprovalsTray() {
  const t = useT();
  const { policy } = useConnectorPolicy();
  const [pending, setPending] = useState<PendingConnectorApproval[]>([]);
  const [busy, setBusy] = useState(false);
  // Collapsed parks the tray as a single header line (the steer queue's
  // fold), so a batch the user isn't ready to answer stops covering the
  // composer corner. A fresh batch re-expands: new approvals demand eyes.
  const [collapsed, setCollapsed] = useState(false);
  // Rows whose full request detail is open (clicking a row toggles it); the
  // clamped one-line preview un-clamps to the whole redacted payload.
  const [expandedIds, setExpandedIds] = useState<ReadonlySet<string>>(new Set());
  const previousCount = useRef(0);
  const mounted = useRef(true);
  // The list clips to a max-height once a batch is long; the shared scroll
  // fade signals that more approvals are hidden below (spec/scroll-fade.md).
  const listRef = useRef<HTMLUListElement>(null);
  const fade = useScrollFade(listRef);

  const refresh = useCallback(async () => {
    try {
      const next = await connectorApprovalsPending();
      if (mounted.current) setPending(next);
    } catch {
      // The bridge may not be up yet; the change event will re-drive us.
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    let unlisten: (() => void) | undefined;
    let aborted = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen(CONNECTOR_APPROVALS_CHANGED_EVENT, () => void refresh()))
      .then((cleanup) => {
        if (aborted) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => {});
    return () => {
      mounted.current = false;
      aborted = true;
      unlisten?.();
    };
  }, [refresh]);

  const respondOne = useCallback(
    async (approvalId: string, approve: boolean) => {
      setBusy(true);
      try {
        await connectorApprovalRespond({ approvalId, approve });
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [refresh],
  );

  const respondAll = useCallback(
    async (approve: boolean) => {
      // Answer only the actions currently on screen. An action enqueued after
      // this snapshot has not been reviewed, so it must stay pending rather than
      // be swept into a bulk approve.
      const approvalIds = pending.map((item) => item.approvalId);
      if (approvalIds.length === 0) return;
      setBusy(true);
      try {
        await connectorApprovalsRespondAll({ approve, approvalIds });
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [pending, refresh],
  );

  // The list grows and shrinks as approvals arrive or are answered without a
  // scroll or resize, so nudge the shared fade to re-measure on each change.
  useEffect(() => {
    // Reading the length makes the change signal explicit to the hook linter.
    void pending.length;
    const id = requestAnimationFrame(fade.update);
    return () => cancelAnimationFrame(id);
  }, [pending.length, fade.update]);

  // Dev console driver (window.__connectorApprovals) that parks synthetic
  // approvals in the tray so its styling can be inspected without a live
  // routine proposing actions.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("../../lib/connector-approvals-demo").then(({ registerConnectorApprovalsDemo }) => {
      if (cancelled) return;
      ({ dispose } = registerConnectorApprovalsDemo({ setPending }));
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  // Re-expand when a batch arrives on an empty tray; a collapse mid-batch
  // sticks until the queue drains.
  useEffect(() => {
    if (pending.length > 0 && previousCount.current === 0) setCollapsed(false);
    previousCount.current = pending.length;
  }, [pending.length]);

  if (pending.length === 0) return null;

  // The providers with work in the queue, deduped in arrival order — the
  // header's overlapping logo stack (the avatar-group treatment) says at a
  // glance whose actions are waiting, especially while collapsed.
  const stackProviders = [
    ...new Set(
      pending
        .map((item) => (policy ? providerFromServer(policy, item.server) : null))
        .filter((provider): provider is NonNullable<typeof provider> => provider !== null),
    ),
  ].slice(0, 3);

  // Dev-only: a real queue stacks one mark per connected provider (Google
  // and Linear can both park actions). The demo driver sets
  // window.__connectorApprovalsStack to preview the overlapping-logo
  // treatment with 2 to 3 marks; production ignores it and shows one mark
  // per distinct provider (guarded on import.meta.env.DEV).
  const demoStackCount =
    import.meta.env.DEV && stackProviders.length > 0
      ? Number(
          (window as unknown as { __connectorApprovalsStack?: number }).__connectorApprovalsStack,
        ) || 0
      : 0;
  const stackMarks =
    demoStackCount > 0
      ? Array.from({ length: Math.min(demoStackCount, 3) }, () => stackProviders[0])
      : stackProviders;

  return (
    <aside
      className="connector-approvals"
      aria-label={t("settingsPanels.approvals.label")}
      // A status role fits a passive, self-updating queue better than a live
      // region alert.
      role="status"
    >
      <header className="connector-approvals-header">
        <button
          type="button"
          className="connector-approvals-trigger"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((current) => !current)}
        >
          <span className="connector-approvals-stack" aria-hidden>
            {stackMarks.length > 0 ? (
              stackMarks.map((provider, index) => (
                <span key={`${provider}-${index}`} className="connector-approvals-stack-mark">
                  <ConnectorProviderIcon provider={provider} size={10} />
                </span>
              ))
            ) : (
              <span className="connector-approvals-stack-mark">
                <IconChecklist size={10} aria-hidden />
              </span>
            )}
          </span>
          {t("settingsPanels.approvals.needed")}
          {collapsed ? <span className="status-pill">{pending.length}</span> : null}
        </button>
        <span className="connector-approvals-header-actions">
          {!collapsed && pending.length > 1 ? (
            <span className="connector-approvals-bulk">
              <button
                type="button"
                className="btn btn-ghost connector-approvals-deny"
                disabled={busy}
                onClick={() => void respondAll(false)}
              >
                {t("settingsPanels.approvals.denyAll")}
              </button>
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => void respondAll(true)}
              >
                {t("settingsPanels.approvals.approveAll")}
              </button>
            </span>
          ) : null}
          <button
            type="button"
            className="connector-approvals-chevron-button"
            aria-label={
              collapsed
                ? t("settingsPanels.approvals.expand")
                : t("settingsPanels.approvals.collapse")
            }
            aria-expanded={!collapsed}
            onClick={() => setCollapsed((current) => !current)}
          >
            <IconChevronDownSmall
              size={13}
              className="connector-approvals-chevron"
              data-expanded={!collapsed}
              aria-hidden
            />
          </button>
        </span>
      </header>
      {collapsed ? null : (
        <ul className="connector-approvals-list scroll-fade-mask" ref={listRef} {...fade.props}>
          {pending.map((item) => {
            const provider = policy ? providerFromServer(policy, item.server) : null;
            const toolLabel = policy
              ? actionToolLabel(policy, item.tool, item.server)
              : item.tool.replace(/[_-]/g, " ");
            const summary = item.summary || toolLabel;
            const expanded = expandedIds.has(item.approvalId);
            const info = (
              <>
                <span className="connector-approvals-summary">{summary}</span>
                <span className="connector-approvals-meta">
                  {toolLabel} · {item.accountEmail}
                </span>
                {item.argsPreview ? (
                  <span className="connector-approvals-preview">{item.argsPreview}</span>
                ) : null}
              </>
            );
            return (
              // The redacted args preview (redaction happens in Rust) shows as
              // one ellipsized line; clicking the row un-clamps it to the full
              // request detail.
              <li
                key={item.approvalId}
                className="connector-approvals-item"
                data-expanded={expanded || undefined}
              >
                <span className="connector-approvals-mark" aria-hidden>
                  {provider ? (
                    <ConnectorProviderIcon provider={provider} size={14} />
                  ) : (
                    <IconChecklist size={14} aria-hidden />
                  )}
                </span>
                {item.argsPreview ? (
                  <button
                    type="button"
                    className="connector-approvals-info"
                    aria-expanded={expanded}
                    title={expanded ? undefined : t("settingsPanels.approvals.showFull")}
                    onClick={() =>
                      setExpandedIds((current) => {
                        const next = new Set(current);
                        if (next.has(item.approvalId)) next.delete(item.approvalId);
                        else next.add(item.approvalId);
                        return next;
                      })
                    }
                  >
                    {info}
                  </button>
                ) : (
                  <div className="connector-approvals-info">{info}</div>
                )}
                <div className="connector-approvals-actions">
                  <button
                    type="button"
                    className="connector-approvals-item-deny"
                    aria-label={t("settingsPanels.approvals.denyItem", { summary })}
                    title={t("settingsPanels.approvals.deny")}
                    disabled={busy}
                    onClick={() => void respondOne(item.approvalId, false)}
                  >
                    <IconCrossSmall size={14} aria-hidden />
                  </button>
                  <button
                    type="button"
                    className="connector-approvals-item-approve"
                    aria-label={t("settingsPanels.approvals.approveItem", { summary })}
                    title={t("settingsPanels.approvals.approve")}
                    disabled={busy}
                    onClick={() => void respondOne(item.approvalId, true)}
                  >
                    <IconCheckmark2Small size={14} aria-hidden />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </aside>
  );
}
