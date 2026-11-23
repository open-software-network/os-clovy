/**
 * Pure, render-free view logic for the private connectors (Google, Linear)
 * in local mode: scope-bundle metadata, account status labels, trust-mode
 * metadata and earned-autonomy gating, trust-mode tool composition, and
 * event-trigger metadata.
 *
 * Kept separate from the React components and the Tauri bindings so all of it
 * is unit-testable without a Tauri runtime. Copy follows Clovy's UI rules.
 */

import { IconBolt } from "central-icons/IconBolt";
import { IconChecklist } from "central-icons/IconChecklist";
import { IconEyeOpen } from "central-icons/IconEyeOpen";
import { getInterfaceLocale } from "../i18n/locale";
import { formatList, type MessageKey, t } from "../i18n/translate";
import { errorCode } from "./errors";
import type {
  ConnectorAccountStatus,
  ConnectorPolicyCatalog,
  ConnectorProvider,
  ConnectorScopeBundle,
  ConnectorTriggerKind,
  RoutineTrustMode,
} from "./tauri";

export const UNRESTRICTED_AGENT_TOOLSETS = [
  "terminal",
  "file",
  "code_execution",
  "web",
  "vision",
  "tts",
  "skills",
  "todo",
  "memory",
  "context_engine",
  "session_search",
  "delegation",
] as const;

// ---------------------------------------------------------------------------
// Scope bundles
// ---------------------------------------------------------------------------

export type ConnectorBundleMeta = {
  /** Checkbox label in the connect dialog. Sentence case. */
  label: string;
  /** One-line supporting copy under the label. */
  description: string;
  /** Short feature phrase for "This routine can: ..." summaries. */
  feature: string;
};

/** Copy fields that read the interface language on every access, so the
 * frozen maps below keep their shape while following a language switch. */
function localizedFields<K extends string>(keys: Record<K, MessageKey>): Record<K, string> {
  const fields = {} as Record<K, string>;
  for (const [field, key] of Object.entries(keys) as [K, MessageKey][]) {
    Object.defineProperty(fields, field, { enumerable: true, get: () => t(key) });
  }
  return fields;
}

function bundleCopy(name: string): ConnectorBundleMeta {
  return localizedFields({
    label: `lib.connectors.${name}.label` as MessageKey,
    description: `lib.connectors.${name}.description` as MessageKey,
    feature: `lib.connectors.${name}.feature` as MessageKey,
  });
}

/** Renderer-only copy. Bundle ownership and scopes come from Rust. */
export const BUNDLE_META: Readonly<Partial<Record<ConnectorScopeBundle, ConnectorBundleMeta>>> =
  Object.freeze({
    gmail_read: bundleCopy("gmailRead"),
    gmail_draft: bundleCopy("gmailDraft"),
    gmail_modify: bundleCopy("gmailModify"),
    gmail_send: bundleCopy("gmailSend"),
    calendar_read: bundleCopy("calendarRead"),
    calendar_events: bundleCopy("calendarEvents"),
    linear_read: bundleCopy("linearRead"),
    linear_write: bundleCopy("linearWrite"),
    github_read: bundleCopy("githubRead"),
    github_write: bundleCopy("githubWrite"),
  });

export function bundleMeta(bundle: ConnectorScopeBundle): ConnectorBundleMeta {
  return (
    BUNDLE_META[bundle] ?? {
      label: bundle.replace(/_/g, " "),
      description: t("lib.connectors.bundleFallbackDescription"),
      feature: bundle.replace(/_/g, " "),
    }
  );
}

/** The feature bundles a provider's connect dialog offers, in display
 * order. */
export function bundlesForProvider(
  policy: ConnectorPolicyCatalog,
  provider: ConnectorProvider,
): ConnectorScopeBundle[] {
  return policy.scopeBundles
    .filter((bundle) => bundle.provider === provider)
    .map((bundle) => bundle.id);
}

export function defaultBundlesForProvider(
  policy: ConnectorPolicyCatalog,
  provider: ConnectorProvider,
): ConnectorScopeBundle[] {
  return (
    policy.providers.find((definition) => definition.id === provider)?.defaultBundles ?? []
  ).slice();
}

/** True when the granted scope set satisfies `needed`, directly or because a
 * broader granted scope implies it. */
function grantsScope(
  policy: ConnectorPolicyCatalog,
  granted: Set<string>,
  needed: string,
): boolean {
  if (granted.has(needed)) return true;
  for (const held of granted) {
    if (policy.scopeImplications.find((entry) => entry.held === held)?.grants.includes(needed)) {
      return true;
    }
  }
  return false;
}

/** Maps an account's granted scope identifiers back to the feature bundles it
 * explicitly holds, in registry order, scoped to that account's provider so a
 * hypothetical scope-string collision across providers can never cross-match.
 * Exact match (not superset): this drives display and reconnect, which should
 * reflect what was actually granted, not what a broader scope could stand in
 * for. Unknown identifiers (identity scopes, future grants) are ignored: the
 * UI shows features, not raw scopes. */
export function bundlesFromScopes(
  policy: ConnectorPolicyCatalog,
  scopeUrls: string[],
  provider: ConnectorProvider,
): ConnectorScopeBundle[] {
  const granted = new Set(scopeUrls);
  return policy.scopeBundles
    .filter((bundle) => bundle.provider === provider)
    .filter((bundle) => bundle.scopeIds.every((scope) => granted.has(scope)))
    .map((bundle) => bundle.id);
}

/** "Read mail, draft replies and calendar" — the human feature list an
 * account's grants render as. */
export function grantedFeatureLabels(
  policy: ConnectorPolicyCatalog,
  scopeUrls: string[],
  provider: ConnectorProvider,
): string[] {
  return bundlesFromScopes(policy, scopeUrls, provider).map((bundle) => bundleMeta(bundle).label);
}

/** True when the account's granted scopes already cover every bundle a routine
 * needs, counting a broader granted scope as covering a narrower need (so a
 * read-only briefing runs on an account that granted calendar write). */
export function scopesCoverBundles(
  policy: ConnectorPolicyCatalog,
  scopeUrls: string[],
  bundles: readonly ConnectorScopeBundle[],
): boolean {
  const granted = new Set(scopeUrls);
  return bundles.every((bundleId) => {
    const bundle = policy.scopeBundles.find((entry) => entry.id === bundleId);
    return Boolean(bundle?.scopeIds.every((scope) => grantsScope(policy, granted, scope)));
  });
}

// ---------------------------------------------------------------------------
// Account status
// ---------------------------------------------------------------------------

export type ConnectorStatusMeta = {
  label: string;
  tone: "ok" | "attention";
  blurb: string;
};

const STATUS_LABELS: Readonly<
  Record<ConnectorAccountStatus, { labelKey: MessageKey; tone: "ok" | "attention" }>
> = Object.freeze({
  connected: { labelKey: "lib.connectors.status.connected", tone: "ok" },
  reconnect_required: { labelKey: "lib.connectors.status.reconnectRequired", tone: "attention" },
  unavailable: { labelKey: "lib.connectors.status.unavailable", tone: "attention" },
});

/** Connected blurb is shared across providers; reconnect names the provider
 * and what it gates ("this account" for Google, "this workspace" for
 * Linear) so the prompt reads correctly for either. */
const RECONNECT_BLURB: Readonly<Record<ConnectorProvider, MessageKey>> = Object.freeze({
  google: "lib.connectors.blurb.reconnectGoogle",
  linear: "lib.connectors.blurb.reconnectLinear",
  notion: "lib.connectors.blurb.reconnectNotion",
  github: "lib.connectors.blurb.reconnectGithub",
});

function accountStatusBlurb(status: ConnectorAccountStatus, provider: ConnectorProvider): string {
  if (status === "reconnect_required") return t(RECONNECT_BLURB[provider]);
  if (status === "unavailable") return t("lib.connectors.blurb.unavailable");
  return t("lib.connectors.blurb.connected");
}

export function accountStatusMeta(
  status: ConnectorAccountStatus,
  provider: ConnectorProvider,
): ConnectorStatusMeta {
  const { labelKey, tone } = STATUS_LABELS[status];
  return { label: t(labelKey), tone, blurb: accountStatusBlurb(status, provider) };
}

/** True for the Rust "connector_not_configured" error: this build ships no
 * Google OAuth client id, so the connect flow cannot start. An expected
 * condition (dev builds), not a failure toast. */
export function isConnectorNotConfiguredError(err: unknown): boolean {
  return errorCode(err) === "connector_not_configured";
}

// ---------------------------------------------------------------------------
// Trust modes
// ---------------------------------------------------------------------------

export type TrustModeMeta = {
  label: string;
  description: string;
  icon: typeof IconBolt;
};

/** Whether a trust save changed the rendered connector runtime enough to need a
 * restart. Two cases: a provider was added or removed (auto server names
 * differ), or an autonomous routine's granted tools changed within a provider.
 * In the second case the server name is unchanged but `routine_trust_set`
 * re-mints the grant token and rewrites `tools.include`, so the live MCP process
 * would keep a stale token/filter (added tools missing, previously granted tools
 * falling back to approval) until some later restart. */
export function autonomyRuntimeNeedsRestart(input: {
  previousServers: readonly string[];
  nextServers: readonly string[];
  trustMode: RoutineTrustMode;
  previousTools: readonly string[];
  nextTools: readonly string[];
}): boolean {
  const differ = (a: readonly string[], b: readonly string[]) =>
    JSON.stringify([...a].sort()) !== JSON.stringify([...b].sort());
  return (
    differ(input.previousServers, input.nextServers) ||
    (input.trustMode === "autonomous" && differ(input.previousTools, input.nextTools))
  );
}

export const TRUST_MODE_META: Readonly<Record<RoutineTrustMode, TrustModeMeta>> = Object.freeze({
  read_only: Object.assign(
    localizedFields({
      label: "lib.connectors.trust.readOnly.label",
      description: "lib.connectors.trust.readOnly.description",
    }),
    { icon: IconEyeOpen },
  ),
  approval: Object.assign(
    localizedFields({
      label: "lib.connectors.trust.approval.label",
      description: "lib.connectors.trust.approval.description",
    }),
    { icon: IconChecklist },
  ),
  autonomous: Object.assign(
    localizedFields({
      label: "lib.connectors.trust.autonomous.label",
      description: "lib.connectors.trust.autonomous.description",
    }),
    { icon: IconBolt },
  ),
});

/**
 * Runs completed under approval mode before autonomous unlocks (earned
 * autonomy). The gate counts successful runs while the routine is in approval
 * mode, not individually approved actions: the connector proxy is session
 * blind (it gates on the grant token alone, with no run or job identity), so it
 * cannot attribute a specific approval to a specific run. The copy below says
 * "under approval" rather than "approved" to match what is actually counted.
 */
export function canSelectAutonomous(policy: ConnectorPolicyCatalog, runCount: number): boolean {
  return runCount >= policy.earnedAutonomyMinApprovalRuns;
}

/** Helper text under the trust picker while autonomy is still locked:
 * "Runs 2 more times under approval to unlock autonomous". */
export function autonomyUnlockHint(policy: ConnectorPolicyCatalog, runCount: number): string {
  const remaining = Math.max(0, policy.earnedAutonomyMinApprovalRuns - runCount);
  if (remaining === 0) return t("lib.connectors.autonomyUnlocked");
  return t("lib.connectors.autonomyUnlockHint", { count: remaining });
}

/** Progress label for the detail page: "Run 2 of 3 under approval before
 * autonomy unlocks". Clamped once the threshold is met. */
export function autonomyProgressLabel(policy: ConnectorPolicyCatalog, runCount: number): string {
  if (canSelectAutonomous(policy, runCount)) return t("lib.connectors.autonomyProgressDone");
  const threshold = policy.earnedAutonomyMinApprovalRuns;
  const next = Math.min(runCount + 1, threshold);
  return t("lib.connectors.autonomyProgress", { next, threshold });
}

// ---------------------------------------------------------------------------
// Trust-mode toolset composition
// ---------------------------------------------------------------------------

/** Renderer projection of one native-owned connector action tool. */
export type ConnectorActionTool = {
  id: string;
  server: string;
  provider: ConnectorProvider;
  label: string;
  grantable: boolean;
};

/** Labels only. Tool/server ownership and grantability come from Rust. */
const ACTION_TOOL_LABELS: Readonly<Record<string, string>> = Object.freeze(
  localizedFields({
    "june_gmail_actions:create_draft": "lib.connectors.action.gmailCreateDraft",
    "june_gmail_actions:send_email": "lib.connectors.action.gmailSendEmail",
    "june_gmail_actions:modify_labels": "lib.connectors.action.gmailModifyLabels",
    "june_gmail_actions:archive": "lib.connectors.action.gmailArchive",
    "june_gcal_actions:create_event": "lib.connectors.action.gcalCreateEvent",
    "june_gcal_actions:respond_to_invite": "lib.connectors.action.gcalRespondToInvite",
    "june_linear_actions:create_issue": "lib.connectors.action.linearCreateIssue",
    "june_linear_actions:update_issue": "lib.connectors.action.linearUpdateIssue",
    "june_linear_actions:add_comment": "lib.connectors.action.linearAddComment",
    "june_linear_actions:create_project_update": "lib.connectors.action.linearCreateProjectUpdate",
    "june_notion_actions:notion-create-pages": "lib.connectors.action.notionCreatePages",
    "june_notion_actions:notion-update-page": "lib.connectors.action.notionUpdatePage",
    "june_github_actions:create_issue": "lib.connectors.action.githubCreateIssue",
    "june_github_actions:update_issue": "lib.connectors.action.githubUpdateIssue",
    "june_github_actions:add_comment": "lib.connectors.action.githubAddComment",
  }),
);

export function connectorActionTools(policy: ConnectorPolicyCatalog): ConnectorActionTool[] {
  return policy.actionTools.map((tool) => ({
    ...tool,
    label: ACTION_TOOL_LABELS[`${tool.server}:${tool.id}`] ?? tool.id.replace(/[_-]/g, " "),
  }));
}

export function grantableConnectorActionTools(
  policy: ConnectorPolicyCatalog,
): ConnectorActionTool[] {
  return connectorActionTools(policy).filter((tool) => tool.grantable);
}

/** A scheduled run that counts toward earned autonomy: one the routine
 * finished without failing. Active runs and error/cancelled runs do not count
 * ("run correctly under approval"). Field names cover both the snake_case and
 * camelCase shapes the session record can arrive in. */
export function isCreditableRun(run: {
  active?: boolean;
  is_active?: boolean;
  status?: string;
  ended_at?: string | null;
  endedAt?: string | null;
}): boolean {
  if (run.active || run.is_active) return false;
  const ended = run.ended_at ?? run.endedAt;
  if (!ended) return false;
  const status = (run.status ?? "").toLowerCase();
  return status !== "failed" && status !== "error" && status !== "cancelled";
}

/** A human label for a connector action tool id, for the approvals surface.
 * Pass the originating server when known: colliding ids resolve to that
 * provider's label. Falls back to a spaced form of the raw name for any
 * unmapped tool. */
export function actionToolLabel(
  policy: ConnectorPolicyCatalog,
  tool: string,
  server?: string,
): string {
  if (server) {
    const qualified = ACTION_TOOL_LABELS[`${server}:${tool}`];
    if (qualified) {
      return qualified;
    }
  }
  const firstIdentity = policy.actionTools.find((entry) => entry.id === tool);
  if (firstIdentity) {
    return (
      ACTION_TOOL_LABELS[`${firstIdentity.server}:${firstIdentity.id}`] ??
      tool.replace(/[_-]/g, " ")
    );
  }
  return tool.replace(/[_-]/g, " ");
}

/** The provider behind a connector MCP server name, for provider marks on the
 * approvals surface. Null for non-connector servers. */
export function providerFromServer(
  policy: ConnectorPolicyCatalog,
  server: string,
): "google" | "linear" | "notion" | "github" | null {
  return (
    policy.serverOwnerPrefixes.find((definition) => server.startsWith(definition.prefix))
      ?.provider ?? null
  );
}

/**
 * Composes a routine's enabled_toolsets for a trust mode, per the connectors
 * design:
 * - read_only: the base list (sandboxed cron default or the unrestricted
 *   override) plus the read servers only;
 * - approval: read servers plus the actions servers (calls park for
 *   approval in the Rust proxy);
 * - autonomous: read servers plus the per-job auto servers minted by
 *   routine_trust_set — the actions servers are swapped OUT, so anything
 *   not granted stays unavailable rather than silently parking.
 */
export function routineToolsetsFor(
  policy: ConnectorPolicyCatalog,
  trust: RoutineTrustMode,
  options: {
    unrestricted: boolean;
    autonomousServers?: string[];
    routineBrowserServer?: string;
  },
): string[] {
  const base = options.unrestricted
    ? UNRESTRICTED_AGENT_TOOLSETS
    : policy.routine.sandboxedBaseToolsets;
  const toolsets = [...base, ...policy.routine.readToolsets];
  if (trust === "approval") toolsets.push(...policy.routine.actionToolsets);
  if (trust === "autonomous") toolsets.push(...(options.autonomousServers ?? []));
  if (options.routineBrowserServer) toolsets.push(options.routineBrowserServer);
  return [...new Set(toolsets)];
}

/**
 * Derives the trust mode a stored job's toolset override implies, for the
 * list badge — no per-row Tauri round trip. Returns null for routines with
 * no connector toolsets at all (nothing to badge).
 */
export function routineTrustModeFromToolsets(
  policy: ConnectorPolicyCatalog,
  enabledToolsets: string[] | undefined,
): RoutineTrustMode | null {
  const toolsets = enabledToolsets ?? [];
  if (
    toolsets.some((toolset) =>
      policy.routine.autonomousServerPrefixes.some((prefix) => toolset.startsWith(prefix)),
    )
  ) {
    return "autonomous";
  }
  if (toolsets.some((toolset) => policy.routine.actionToolsets.includes(toolset))) {
    return "approval";
  }
  if (toolsets.some((toolset) => policy.routine.readToolsets.includes(toolset))) {
    return "read_only";
  }
  return null;
}

// ---------------------------------------------------------------------------
// Event triggers
// ---------------------------------------------------------------------------

export type TriggerMeta = {
  label: string;
  description: string;
  /** Config keys the trigger's kind carries in ConnectorTrigger.config. */
  configFields: string[];
};

export const TRIGGER_META: Readonly<Record<ConnectorTriggerKind, TriggerMeta>> = Object.freeze({
  email_received: Object.assign(
    localizedFields({
      label: "lib.connectors.trigger.emailReceived.label",
      description: "lib.connectors.trigger.emailReceived.description",
    }),
    { configFields: [] },
  ),
  event_upcoming: Object.assign(
    localizedFields({
      label: "lib.connectors.trigger.eventUpcoming.label",
      description: "lib.connectors.trigger.eventUpcoming.description",
    }),
    { configFields: ["leadMinutes", "externalOnly"] },
  ),
});

/** The routine editor's "When" model: a plain schedule, or a connector event
 * trigger. Kept out of ScheduleDraft on purpose: events never encode into
 * the cron string; the daemon fires the active routine directly. */
export type TriggerDraft =
  | { source: "schedule" }
  | { source: "email_received" }
  | { source: "event_upcoming"; leadMinutes: number; externalOnly: boolean };

export const DEFAULT_EVENT_LEAD_MINUTES = 30;

/**
 * The connector scope bundle a connector event trigger's daemon must be able to
 * call on the account it subscribes on: a Gmail read to poll for new mail, a
 * calendar read to poll upcoming events. Schedules poll nothing, so they need
 * none. Drives the create/edit gate so a trigger can't be saved against an
 * account whose token lacks the scope the daemon will call, which would leave
 * the routine silently never firing (the Gmail history / calendar list call
 * fails on the missing scope).
 */
export function triggerRequiredBundles(
  policy: ConnectorPolicyCatalog,
  trigger: TriggerDraft,
): readonly ConnectorScopeBundle[] {
  if (trigger.source === "schedule") return [];
  return (
    policy.triggers.find((definition) => definition.id === trigger.source)?.requiredBundles ?? []
  );
}

/**
 * A one-line warning when the account a connector trigger would subscribe on is
 * connected but lacks the scope its daemon needs, naming the missing access and
 * where to add it. Returns null when the trigger needs no scope, the account
 * already covers it, or no account is connected (the picker shows its own
 * "connect an account" notice in that case).
 */
export function triggerScopeWarning(
  policy: ConnectorPolicyCatalog,
  trigger: TriggerDraft,
  accountScopes: string[] | null,
): string | null {
  const bundles = triggerRequiredBundles(policy, trigger);
  if (bundles.length === 0) return null;
  if (accountScopes == null) return null;
  if (scopesCoverBundles(policy, accountScopes, bundles)) return null;
  const labels = bundles.map((bundle) => bundleMeta(bundle).label.toLowerCase());
  const features =
    getInterfaceLocale() === "en"
      ? labels.join(" and ")
      : formatList(labels, { type: "conjunction" });
  return t("lib.connectors.triggerScopeWarning", { features });
}

/** Contract check for the renderer-only presentation maps. */
export function missingConnectorPresentationIds(policy: ConnectorPolicyCatalog): string[] {
  const missingBundles = policy.scopeBundles
    .filter((bundle) => !BUNDLE_META[bundle.id])
    .map((bundle) => `bundle:${bundle.id}`);
  const missingActions = policy.actionTools
    .filter((tool) => !ACTION_TOOL_LABELS[`${tool.server}:${tool.id}`])
    .map((tool) => `action:${tool.server}:${tool.id}`);
  return [...missingBundles, ...missingActions];
}

/**
 * The schedule an event-triggered routine is created with. Event routines
 * still need a persisted schedule, so they get a far-future one-time value.
 * They remain active for connector wakes, while the distant date guarantees
 * the timer scheduler itself never runs them.
 */
export function eventTriggerScheduleDraft(): { schedule: string } {
  return { schedule: "2099-01-01T09:00:00Z" };
}

/** Builds the config payload connector_trigger_set expects for a draft. */
export function triggerConfigFromDraft(
  draft: Exclude<TriggerDraft, { source: "schedule" }>,
): Record<string, unknown> {
  if (draft.source === "event_upcoming") {
    return { leadMinutes: draft.leadMinutes, externalOnly: draft.externalOnly };
  }
  return {};
}
