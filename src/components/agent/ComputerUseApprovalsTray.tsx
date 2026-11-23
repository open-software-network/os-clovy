import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconStop } from "central-icons/IconStop";
import { IconTelevision } from "central-icons/IconTelevision";
import { useCallback, useEffect, useRef, useState } from "react";
import { clockTimeOptions, intlLocale, t as translate, useT, useTRich } from "../../i18n";
import { messageFromError } from "../../lib/errors";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  COMPUTER_USE_APPROVALS_CHANGED_EVENT,
  type PendingComputerUseApprovalDto,
  computerUseApprovalsPending,
  computerUseCaptureSrc,
  computerUseStop,
  respondComputerUseApproval,
} from "../../lib/tauri";

function expiryLabel(expiresAtMs: number) {
  return new Intl.DateTimeFormat(intlLocale(), {
    ...clockTimeOptions(),
  }).format(new Date(expiresAtMs));
}

function approvalHeading(pending: PendingComputerUseApprovalDto[]) {
  const targetApps = [
    ...new Set(pending.map((item) => item.targetApp.trim()).filter((target) => target.length > 0)),
  ];
  if (targetApps.length === 1)
    return translate("agent.computerUse.headingApp", { app: targetApps[0] });
  if (targetApps.length > 1) {
    return translate("agent.computerUse.headingApps", { count: targetApps.length });
  }
  return translate("agent.computerUse.headingUnknown");
}

/** Always-mounted decision surface for the app-owned Computer use broker.
 * Approval is scoped to one target app and expires with the current task. */
export function ComputerUseApprovalsTray() {
  const t = useT();
  const tr = useTRich();
  const [pending, setPending] = useState<PendingComputerUseApprovalDto[]>([]);
  const [busy, setBusy] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [error, setError] = useState<string>();
  const mounted = useRef(true);
  const previousCount = useRef(0);
  const listRef = useRef<HTMLUListElement>(null);
  const fade = useScrollFade(listRef);

  const refresh = useCallback(async () => {
    try {
      const next = await computerUseApprovalsPending();
      if (mounted.current) setPending(next);
    } catch {
      // Startup and shutdown can briefly leave the native bridge unavailable.
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    let unlisten: (() => void) | undefined;
    let aborted = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen(COMPUTER_USE_APPROVALS_CHANGED_EVENT, () => void refresh()))
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

  useEffect(() => {
    if (pending.length > 0 && previousCount.current === 0) setCollapsed(false);
    previousCount.current = pending.length;
  }, [pending.length]);

  useEffect(() => {
    void pending.length;
    const id = requestAnimationFrame(fade.update);
    return () => cancelAnimationFrame(id);
  }, [pending.length, fade.update]);

  const respond = useCallback(
    async (approvalId: string, approve: boolean) => {
      setBusy(true);
      setError(undefined);
      try {
        await respondComputerUseApproval({ approvalId, approve });
      } catch (nextError) {
        if (mounted.current) setError(messageFromError(nextError));
      } finally {
        if (mounted.current) setBusy(false);
        void refresh();
      }
    },
    [refresh],
  );

  const stop = useCallback(async () => {
    setBusy(true);
    setError(undefined);
    try {
      await computerUseStop();
    } catch (nextError) {
      if (mounted.current) setError(messageFromError(nextError));
    } finally {
      if (mounted.current) setBusy(false);
      void refresh();
    }
  }, [refresh]);

  if (pending.length === 0) return null;

  return (
    <aside
      className="connector-approvals computer-use-approvals"
      aria-label={t("agent.computerUse.label")}
    >
      <header className="connector-approvals-header">
        <button
          type="button"
          className="connector-approvals-trigger"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((current) => !current)}
        >
          <span className="connector-approvals-stack-mark" aria-hidden>
            <IconTelevision size={11} />
          </span>
          {approvalHeading(pending)}
          {collapsed ? <span className="status-pill">{pending.length}</span> : null}
        </button>
        <span className="connector-approvals-header-actions">
          {!collapsed ? (
            <button
              type="button"
              className="btn btn-ghost computer-use-stop-button"
              disabled={busy}
              onClick={() => void stop()}
            >
              <IconStop size={12} aria-hidden />
              {t("agent.computerUse.stop")}
            </button>
          ) : null}
          <button
            type="button"
            className="connector-approvals-chevron-button"
            aria-label={collapsed ? t("agent.computerUse.expand") : t("agent.computerUse.collapse")}
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
        <ul className="computer-use-approvals-list scroll-fade-mask" ref={listRef} {...fade.props}>
          {pending.map((item) => (
            <li key={item.approvalId} className="computer-use-approval-card">
              {item.capturePath ? (
                <img
                  className="computer-use-approval-capture"
                  src={computerUseCaptureSrc(item.capturePath)}
                  alt={t("agent.computerUse.captureAlt", { app: item.targetApp })}
                />
              ) : null}
              <div className="computer-use-approval-copy">
                <strong>{item.summary}</strong>
                <span>
                  {tr("agent.computerUse.expiresAt", {
                    time: (
                      <time dateTime={new Date(item.expiresAtMs).toISOString()}>
                        {expiryLabel(item.expiresAtMs)}
                      </time>
                    ),
                  })}
                </span>
              </div>
              <div className="computer-use-approval-actions">
                <button
                  type="button"
                  className="btn btn-ghost"
                  disabled={busy}
                  onClick={() => void respond(item.approvalId, false)}
                >
                  {t("agent.computerUse.deny")}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={busy}
                  onClick={() => void respond(item.approvalId, true)}
                >
                  {t("agent.computerUse.allow")}
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {error ? (
        <p className="computer-use-approval-error" role="alert">
          {error}
        </p>
      ) : null}
    </aside>
  );
}
