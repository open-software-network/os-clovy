import { IconGoogle } from "central-icons/IconGoogle";
import { useEffect, useState } from "react";
import {
  TRUST_MODE_META,
  isConnectorNotConfiguredError,
  scopesCoverBundles,
  triggerRequiredBundles,
  triggerScopeWarning,
  type TriggerDraft,
} from "../../lib/connectors";
import { useConnectorPolicy } from "../../lib/connector-policy";
import { messageFromError } from "../../lib/errors";
import {
  draftFromSchedule,
  scheduleFromDraft,
  type ScheduleDraft,
} from "../../lib/routine-schedule";
import {
  connectorsConnect,
  connectorsList,
  type ConnectorAccount,
  type RoutineTrustMode,
} from "../../lib/tauri";
import { useT } from "../../i18n";
import { BreadcrumbBar } from "../ui/BreadcrumbBar";
import { InlineNotice } from "../ui/InlineNotice";
import { GrowingTextarea } from "./GrowingTextarea";
import { RoutineModePicker } from "./RoutineModePicker";
import { TriggerPicker } from "./TriggerPicker";
import { TrustModePicker } from "./TrustModePicker";
import type { RoutineTemplate } from "./routine-templates";

export type RoutineCreateInput = {
  prompt: string;
  schedule: string;
  name?: string;
  unrestricted: boolean;
  /** Connector trust for the new routine (read only unless chosen). */
  trustMode: RoutineTrustMode;
  autonomousTools: string[];
  /** The "When" choice: a schedule, or a connector event trigger. */
  trigger: TriggerDraft;
  /** Account the event trigger subscribes on (first connected account). */
  triggerAccountId?: string;
  /** Set when installing a connector template, so the create flow knows to
   * persist trust and queue the immediate first run. */
  connectorScopes?: RoutineTemplate["connectorScopes"];
};

type RoutineCreateProps = {
  /** Prefills the editor; the user still reviews and saves explicitly. */
  template?: RoutineTemplate;
  creating: boolean;
  error: string | null;
  onBack: () => void;
  onCreate: (input: RoutineCreateInput) => void;
};

export function RoutineCreate({ template, creating, error, onBack, onCreate }: RoutineCreateProps) {
  const t = useT();
  const { policy } = useConnectorPolicy();
  const [name, setName] = useState(template?.name ?? "");
  const [draft, setDraft] = useState<ScheduleDraft>(() =>
    template ? draftFromSchedule(template.schedule) : { kind: "daily", time: "09:00" },
  );
  const [prompt, setPrompt] = useState(template?.prompt ?? "");
  const [unrestricted, setUnrestricted] = useState(template?.unrestricted ?? false);
  const [trustMode, setTrustMode] = useState<RoutineTrustMode>(template?.trustMode ?? "read_only");
  const [autonomousTools, setAutonomousTools] = useState<string[]>([]);
  const [trigger, setTrigger] = useState<TriggerDraft>(() => {
    const templateTrigger = template?.trigger;
    if (!templateTrigger) return { source: "schedule" };
    if (templateTrigger.kind === "email_received") return { source: "email_received" };
    return {
      source: "event_upcoming",
      leadMinutes: templateTrigger.leadMinutes ?? 30,
      externalOnly: templateTrigger.externalOnly ?? true,
    };
  });
  // null while loading; connector features degrade quietly (plain routines
  // never need an account, and the Rust side may not be present in dev).
  const [accounts, setAccounts] = useState<ConnectorAccount[] | null>(null);
  const [connectBusy, setConnectBusy] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    connectorsList()
      .then((list) => {
        if (!cancelled) setAccounts(list);
      })
      .catch(() => {
        if (!cancelled) setAccounts([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const requiredScopes = template?.connectorScopes;
  // The routine runs against the first connected account (triggerAccountId
  // below, and Clovy registers the same account for its connector tools), so
  // the scope gate must check that exact account. Checking "any account" would
  // enable Create while the routine still polls/calls Google with an account
  // that lacks the scope, silently missing triggers or failing on scope errors.
  const connectedAccount = (accounts ?? []).find(
    (account) => account.provider === "google" && account.status === "connected",
  );
  const scopeGateSatisfied =
    !requiredScopes ||
    (policy != null &&
      connectedAccount != null &&
      scopesCoverBundles(policy, connectedAccount.scopes, requiredScopes));
  // A connector trigger must run on an account that holds the scope its daemon
  // polls (Gmail read for new mail, calendar read for upcoming events). Checking
  // "any account connected" is not enough: a calendar-only account can't back an
  // email_received trigger, so the daemon's Gmail history call fails and the
  // routine silently never fires. A broader granted scope still counts (an
  // account with calendar write backs an upcoming-event trigger).
  const triggerBundles = policy ? triggerRequiredBundles(policy, trigger) : [];
  const triggerScopeSatisfied =
    trigger.source === "schedule" ||
    (policy != null &&
      triggerBundles.length > 0 &&
      connectedAccount != null &&
      scopesCoverBundles(policy, connectedAccount.scopes, triggerBundles));
  const connectorPolicyRequired =
    Boolean(requiredScopes) || trigger.source !== "schedule" || trustMode !== "read_only";
  const blocked =
    (connectorPolicyRequired && policy == null) || !scopeGateSatisfied || !triggerScopeSatisfied;

  async function connectForTemplate() {
    if (!requiredScopes || !policy || connectBusy) return;
    setConnectBusy(true);
    setConnectError(null);
    try {
      await connectorsConnect({ scopes: requiredScopes });
      setAccounts(await connectorsList());
    } catch (err) {
      setConnectError(
        isConnectorNotConfiguredError(err)
          ? t("routines.create.notConfigured")
          : messageFromError(err),
      );
    } finally {
      setConnectBusy(false);
    }
  }

  function submit() {
    if (!prompt.trim() || blocked) return;
    onCreate({
      prompt: prompt.trim(),
      schedule: scheduleFromDraft(draft),
      name: name.trim() || undefined,
      unrestricted,
      trustMode,
      autonomousTools,
      trigger,
      triggerAccountId: connectedAccount?.accountId,
      connectorScopes: requiredScopes,
    });
  }

  return (
    <section className="routine-detail" aria-label={t("routines.newRoutine")}>
      <BreadcrumbBar
        backLabel={t("routines.backToRoutines")}
        onBack={onBack}
        items={[
          { label: t("routines.title"), onClick: onBack },
          { label: name.trim() || t("routines.newRoutine") },
        ]}
        actions={
          <div className="routine-detail-actions">
            <button type="button" className="btn btn-ghost" onClick={onBack}>
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              disabled={!prompt.trim() || creating || blocked}
              onClick={submit}
            >
              {creating ? t("routines.create.creating") : t("common.create")}
            </button>
          </div>
        }
      />

      <div className="routine-detail-content">
        <input
          className="routine-detail-name"
          value={name}
          placeholder={t("routines.routineName")}
          aria-label={t("routines.routineName")}
          onChange={(event) => setName(event.currentTarget.value)}
        />

        {error ? <p className="error-banner">{error}</p> : null}

        {template?.toolSummary ? (
          <p className="routines-tool-summary">
            {t("routines.trust.toolSummary", {
              summary: template.toolSummary,
              trust: TRUST_MODE_META[trustMode].label.toLowerCase(),
            })}
          </p>
        ) : null}

        {requiredScopes && !scopeGateSatisfied ? (
          <InlineNotice
            tone="info"
            aria-label={t("routines.googleAccountRequired")}
            body={connectError ?? t("routines.create.needsAccount")}
            actions={
              <button
                type="button"
                className="btn btn-secondary"
                disabled={connectBusy}
                aria-busy={connectBusy || undefined}
                onClick={() => void connectForTemplate()}
              >
                <IconGoogle size={13} aria-hidden />
                {connectBusy
                  ? t("routines.create.waitingForBrowser")
                  : t("routines.create.connectGoogle")}
              </button>
            }
          />
        ) : null}

        <div className="routine-detail-body">
          <section className="settings-group" aria-labelledby="routine-schedule">
            <h2 id="routine-schedule" className="settings-group-heading">
              {t("routines.section.when")}
            </h2>
            <div className="settings-card">
              <TriggerPicker
                trigger={trigger}
                scheduleDraft={draft}
                hasAccount={Boolean(connectedAccount)}
                scopeWarning={
                  policy
                    ? triggerScopeWarning(policy, trigger, connectedAccount?.scopes ?? null)
                    : null
                }
                onTriggerChange={setTrigger}
                onScheduleChange={setDraft}
              />
            </div>
          </section>

          <section className="settings-group" aria-labelledby="routine-instructions">
            <h2 id="routine-instructions" className="settings-group-heading">
              {t("routines.section.instructions")}
            </h2>
            <GrowingTextarea
              className="routine-detail-instructions"
              value={prompt}
              aria-label={t("routines.section.instructions")}
              placeholder={t("routines.create.instructionsPlaceholder")}
              onChange={(event) => setPrompt(event.currentTarget.value)}
            />
          </section>

          <section className="settings-group" aria-labelledby="routine-access">
            <h2 id="routine-access" className="settings-group-heading">
              {t("routines.section.access")}
            </h2>
            <div className="settings-card">
              <RoutineModePicker unrestricted={unrestricted} onChange={setUnrestricted} />
            </div>
          </section>

          <section className="settings-group" aria-labelledby="routine-trust">
            <h2 id="routine-trust" className="settings-group-heading">
              {t("routines.section.actions")}
            </h2>
            <div className="settings-card">
              <TrustModePicker
                policy={policy}
                value={trustMode}
                runCount={0}
                autonomousTools={autonomousTools}
                onChange={setTrustMode}
                onAutonomousToolsChange={setAutonomousTools}
              />
            </div>
          </section>
        </div>
      </div>
    </section>
  );
}
