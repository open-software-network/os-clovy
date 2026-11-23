import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  createAgentMcpServer,
  connectAgentMcpOauth,
  cancelAgentMcpOauth,
  DEFAULT_AGENT_MCP_SAFETY,
  deleteAgentMcpServer,
  disconnectAgentMcpOauth,
  listAgentMcpServers,
  testAgentMcpServer,
  updateAgentMcpServer,
  type AgentMcpDiagnostic,
  type AgentMcpServerDto,
  type AgentMcpTransport,
} from "../../lib/agent-mcp";
import { type TFunction, useT } from "../../i18n";
import { errorCode, messageFromError } from "../../lib/errors";
import { Dialog } from "../ui/Dialog";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { InlineNotice } from "../ui/InlineNotice";
import { Switch } from "../ui/Switch";

type Draft = {
  name: string;
  transport: AgentMcpTransport;
  command: string;
  args: string;
  url: string;
  env: string;
  headers: string;
  includeTools: string;
  excludeTools: string;
  approvalTools: string;
  requiresApproval: boolean;
  allowSandboxed: boolean;
  oauth: boolean;
};

const EMPTY_DRAFT: Draft = {
  name: "",
  transport: "stdio",
  command: "",
  args: "",
  url: "",
  env: "",
  headers: "",
  includeTools: "",
  excludeTools: "",
  approvalTools: "",
  requiresApproval: true,
  allowSandboxed: true,
  oauth: false,
};

type CheckResult =
  | { kind: "tools"; count: number }
  | { kind: "waiting" | "connected" }
  | { kind: "error"; message: string }
  | { kind: "disconnect"; localCleared: boolean; remoteRevoked: boolean | null };

function checkResultMessage(result: CheckResult | undefined, t: TFunction): string {
  if (!result) return "";
  switch (result.kind) {
    case "tools":
      return t("settingsPanels.mcp.toolsAvailable", { count: result.count });
    case "waiting":
      return t("settingsPanels.mcp.waitingForBrowser");
    case "connected":
      return t("settingsPanels.mcp.oauthConnected");
    case "error":
      return result.message;
    case "disconnect":
      return `${t(result.localCleared ? "settingsPanels.mcp.localCleared" : "settingsPanels.mcp.localClearUnconfirmed")} · ${t(result.remoteRevoked === null ? "settingsPanels.mcp.remoteRevokeUnavailable" : result.remoteRevoked ? "settingsPanels.mcp.remoteRevoked" : "settingsPanels.mcp.remoteRevokeFailed")}`;
  }
}

function parseSecretMap(raw: string, label: string, t: TFunction): Record<string, string> {
  if (!raw.trim()) return {};
  const value: unknown = JSON.parse(raw);
  if (
    !value ||
    Array.isArray(value) ||
    typeof value !== "object" ||
    Object.values(value).some((entry) => typeof entry !== "string")
  ) {
    throw new Error(t("settingsPanels.mcp.secretMapInvalid", { label }));
  }
  return value as Record<string, string>;
}

function hasOauthRevocationHistory(server: AgentMcpServerDto): boolean {
  const history = server.metadata.oauthRevocationHistory;
  return (
    Object.prototype.hasOwnProperty.call(server.metadata, "oauthRevocationHistory") &&
    !(Array.isArray(history) && history.length === 0)
  );
}

function hasOauthConnectCleanup(server: AgentMcpServerDto): boolean {
  if (!Object.prototype.hasOwnProperty.call(server.metadata, "oauthConnectAttempts")) return false;
  const attempts = server.metadata.oauthConnectAttempts;
  if (!Array.isArray(attempts)) return true;
  const fields = ["id", "secretRef", "revision", "epoch", "phase", "superseded"];
  const phases = [
    "authorizing",
    "exchange_pending",
    "received",
    "revocation_failed",
    "revoked_but_local_cleanup_pending",
  ];
  const ids = new Set<string>();
  const references = new Set<string>();
  return attempts.some((attempt: unknown) => {
    if (!attempt || typeof attempt !== "object" || Array.isArray(attempt)) return true;
    const entry = attempt as Record<string, unknown>;
    if (
      Object.keys(entry).some((key) => !fields.includes(key)) ||
      typeof entry.id !== "string" ||
      entry.id.length === 0 ||
      typeof entry.secretRef !== "string" ||
      entry.secretRef.length === 0 ||
      typeof entry.epoch !== "number" ||
      !Number.isSafeInteger(entry.epoch) ||
      entry.epoch < 0 ||
      (entry.revision !== undefined &&
        entry.revision !== null &&
        typeof entry.revision !== "string") ||
      (entry.superseded !== undefined && typeof entry.superseded !== "boolean") ||
      typeof entry.phase !== "string" ||
      !phases.includes(entry.phase) ||
      ids.has(entry.id) ||
      references.has(entry.secretRef) ||
      entry.secretRef === server.secretRef
    ) {
      return true;
    }
    ids.add(entry.id);
    references.add(entry.secretRef);
    return entry.superseded !== true;
  });
}

function diagnosticFromError(error: unknown): AgentMcpDiagnostic | undefined {
  if (!error || typeof error !== "object" || !("details" in error)) return;
  const detail = error.details;
  if (!detail || typeof detail !== "object") return;
  const candidate = detail as Partial<AgentMcpDiagnostic>;
  if (
    candidate.origin === "clovy_managed" &&
    typeof candidate.server === "string" &&
    typeof candidate.phase === "string" &&
    typeof candidate.code === "string" &&
    typeof candidate.state === "string" &&
    typeof candidate.action === "string" &&
    typeof candidate.message === "string"
  ) {
    return candidate as AgentMcpDiagnostic;
  }
}

function diagnosticContext(diagnostic: AgentMcpDiagnostic, t: TFunction): string {
  const phases: Record<string, string> = {
    connect: t("settingsPanels.mcp.phaseConnect"),
    discover: t("settingsPanels.mcp.phaseDiscover"),
    refresh: t("settingsPanels.mcp.phaseRefresh"),
    call: t("settingsPanels.mcp.phaseCall"),
    disconnect: t("settingsPanels.mcp.phaseDisconnect"),
  };
  const actions: Record<string, string> = {
    connect: t("settingsPanels.mcp.actionConnect"),
    reconnect: t("settingsPanels.mcp.actionReconnect"),
    configure: t("settingsPanels.mcp.actionConfigure"),
    check_permission_or_connection: t("settingsPanels.mcp.actionPermission"),
    check_provider_client_approval: t("settingsPanels.mcp.actionClientApproval"),
    review_provider_consent: t("settingsPanels.mcp.actionConsent"),
    revoke_at_provider: t("settingsPanels.mcp.actionRevoke"),
    inspect_server_before_retry: t("settingsPanels.mcp.actionInspect"),
    retry_local_cleanup: t("settingsPanels.mcp.actionCleanup"),
    check_local_storage: t("settingsPanels.mcp.actionStorage"),
    check_connection: t("settingsPanels.mcp.actionConnection"),
    inspect_server: t("settingsPanels.mcp.actionServer"),
    answer_in_chat: t("settingsPanels.mcp.actionAnswer"),
  };
  return [
    phases[diagnostic.phase] ?? diagnostic.phase,
    diagnostic.tool,
    diagnostic.code,
    typeof diagnostic.remoteCode === "number" ? diagnostic.remoteCode : undefined,
    actions[diagnostic.action] ?? diagnostic.action,
  ]
    .filter((value) => value !== undefined && value !== null && value !== "" && value !== "none")
    .join(" · ");
}

export function AgentMcpServersSection() {
  const t = useT();
  const [servers, setServers] = useState<AgentMcpServerDto[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [busyId, setBusyId] = useState<string>();
  const [connectingId, setConnectingId] = useState<string>();
  const connectionOperationId = useRef<string>();
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [addOpen, setAddOpen] = useState(false);
  const [editing, setEditing] = useState<AgentMcpServerDto>();
  const [toDelete, setToDelete] = useState<AgentMcpServerDto>();
  const [saveError, setSaveError] = useState<string>();
  const [testResults, setTestResults] = useState<Record<string, CheckResult | undefined>>({});
  const [diagnostics, setDiagnostics] = useState<Record<string, AgentMcpDiagnostic | null>>({});
  const [health, setHealth] = useState<
    Record<string, "connected" | "reconnect_required" | "unavailable">
  >({});

  const load = useCallback(async () => {
    setLoading(true);
    setError(undefined);
    setHealth({});
    setDiagnostics({});
    setTestResults({});
    try {
      setServers(await listAgentMcpServers());
    } catch (loadError) {
      setError(messageFromError(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function toggle(server: AgentMcpServerDto, enabled: boolean) {
    setBusyId(server.id);
    setError(undefined);
    try {
      const input = { ...server, enabled };
      delete input.diagnostic;
      const updated = await updateAgentMcpServer({
        ...input,
      });
      setServers((current) => current.map((item) => (item.id === server.id ? updated : item)));
      setHealth((current) => {
        const next = { ...current };
        delete next[server.id];
        return next;
      });
      setDiagnostics((current) => ({ ...current, [server.id]: updated.diagnostic ?? null }));
      setTestResults((current) => ({ ...current, [server.id]: undefined }));
    } catch (updateError) {
      await load();
      setError(messageFromError(updateError));
    } finally {
      setBusyId(undefined);
    }
  }

  async function remove(server: AgentMcpServerDto) {
    setBusyId(server.id);
    setError(undefined);
    try {
      const result = await deleteAgentMcpServer(server.id);
      setServers((current) => current.filter((item) => item.id !== server.id));
      if (result.remoteRevoked === false) {
        setError(
          `${t("settingsPanels.mcp.remoteRevokeFailed")}. ${t("settingsPanels.mcp.actionRevoke")}`,
        );
      }
    } catch (deleteError) {
      await load();
      setError(messageFromError(deleteError));
    } finally {
      setBusyId(undefined);
    }
  }

  async function test(server: AgentMcpServerDto) {
    setBusyId(server.id);
    setError(undefined);
    try {
      const tools = await testAgentMcpServer(server.id);
      setDiagnostics((current) => ({ ...current, [server.id]: null }));
      setHealth((current) => ({ ...current, [server.id]: "connected" }));
      setTestResults((current) => ({
        ...current,
        [server.id]: { kind: "tools", count: tools.length },
      }));
    } catch (testError) {
      const diagnostic = diagnosticFromError(testError);
      setDiagnostics((current) => ({ ...current, [server.id]: diagnostic ?? null }));
      setHealth((current) => ({
        ...current,
        [server.id]:
          diagnostic?.state === "reconnect_required" ||
          errorCode(testError) === "agent_mcp_reconnect_required"
            ? "reconnect_required"
            : "unavailable",
      }));
      setTestResults((current) => ({
        ...current,
        [server.id]: { kind: "error", message: diagnostic?.message ?? messageFromError(testError) },
      }));
    } finally {
      setBusyId(undefined);
    }
  }

  async function connectOauth(server: AgentMcpServerDto) {
    const operationId = crypto.randomUUID();
    connectionOperationId.current = operationId;
    setBusyId(server.id);
    setConnectingId(server.id);
    setDiagnostics((current) => ({ ...current, [server.id]: null }));
    setHealth((current) => {
      const next = { ...current };
      delete next[server.id];
      return next;
    });
    setError(undefined);
    setTestResults((current) => ({
      ...current,
      [server.id]: { kind: "waiting" },
    }));
    try {
      const connected = await connectAgentMcpOauth(server.id, operationId);
      setHealth((current) => ({ ...current, [server.id]: "connected" }));
      setServers((current) => current.map((item) => (item.id === connected.id ? connected : item)));
      setTestResults((current) => ({
        ...current,
        [server.id]: { kind: "connected" },
      }));
    } catch (connectError) {
      // A failed reconnect can invalidate a previous grant. Read the effective
      // registry state before offering actions again.
      await load();
      const diagnostic = diagnosticFromError(connectError);
      setDiagnostics((current) => ({ ...current, [server.id]: diagnostic ?? null }));
      setHealth((current) => ({
        ...current,
        [server.id]:
          diagnostic?.state === "reconnect_required" ||
          errorCode(connectError) === "agent_mcp_reconnect_required"
            ? "reconnect_required"
            : "unavailable",
      }));
      setTestResults((current) => ({
        ...current,
        [server.id]: {
          kind: "error",
          message: diagnostic?.message ?? messageFromError(connectError),
        },
      }));
    } finally {
      if (connectionOperationId.current === operationId) connectionOperationId.current = undefined;
      setConnectingId(undefined);
      setBusyId(undefined);
    }
  }

  async function cancelOauth(server: AgentMcpServerDto) {
    try {
      await cancelAgentMcpOauth(server.id, connectionOperationId.current);
    } catch (cancelError) {
      setError(messageFromError(cancelError));
    }
  }

  async function disconnectOauth(server: AgentMcpServerDto) {
    setBusyId(server.id);
    setError(undefined);
    try {
      const result = await disconnectAgentMcpOauth(server.id);
      setDiagnostics((current) => ({ ...current, [server.id]: result.server.diagnostic ?? null }));
      setHealth((current) => {
        const next = { ...current };
        delete next[server.id];
        return next;
      });
      setServers((current) =>
        current.map((item) => (item.id === server.id ? result.server : item)),
      );
      setTestResults((current) => ({
        ...current,
        [server.id]: {
          kind: "disconnect",
          localCleared: result.localCleared,
          remoteRevoked: result.remoteRevoked,
        },
      }));
    } catch (disconnectError) {
      await load();
      setError(messageFromError(disconnectError));
    } finally {
      setBusyId(undefined);
    }
  }

  async function addVercel() {
    setBusyId("vercel-preset");
    setError(undefined);
    try {
      const saved = await createAgentMcpServer({
        name: "Vercel",
        enabled: false,
        transport: "streamable_http",
        args: [],
        url: "https://mcp.vercel.com",
        metadata: { auth: "oauth", preset: "vercel" },
        toolVisibility: { include: [], exclude: [] },
        safety: DEFAULT_AGENT_MCP_SAFETY,
      });
      setServers((current) => [...current, saved].sort((a, b) => a.name.localeCompare(b.name)));
    } catch (createError) {
      setError(messageFromError(createError));
    } finally {
      setBusyId(undefined);
    }
  }

  function openCreate() {
    setEditing(undefined);
    setDraft(EMPTY_DRAFT);
    setSaveError(undefined);
    setAddOpen(true);
  }

  function openEdit(server: AgentMcpServerDto) {
    setEditing(server);
    setDraft({
      name: server.name,
      transport: server.transport,
      command: server.command ?? "",
      args: server.args.join("\n"),
      url: server.url ?? "",
      env: "",
      headers: "",
      includeTools: server.toolVisibility.include.join("\n"),
      excludeTools: server.toolVisibility.exclude.join("\n"),
      approvalTools: server.safety.approvalTools.join("\n"),
      requiresApproval: server.safety.requiresApproval,
      allowSandboxed: server.safety.allowSandboxed,
      oauth: server.metadata.auth === "oauth" || server.metadata.legacyAuth === "oauth",
    });
    setSaveError(undefined);
    setAddOpen(true);
  }

  async function save() {
    setSaveError(undefined);
    try {
      const secretBundle = {
        env: parseSecretMap(draft.env, t("settingsPanels.mcp.environment"), t),
        headers: parseSecretMap(draft.headers, t("settingsPanels.mcp.headers"), t),
      };
      const metadata = { ...(editing?.metadata ?? {}) };
      if (draft.oauth) {
        metadata.auth = "oauth";
      } else {
        delete metadata.auth;
        delete metadata.oauthConnected;
        delete metadata.legacyAuth;
        delete metadata.needsReview;
        delete metadata.migrationWarning;
      }
      const input = {
        id: editing?.id,
        name: draft.name.trim(),
        enabled: draft.oauth ? (editing?.enabled ?? false) : (editing?.enabled ?? true),
        transport: draft.transport,
        command: draft.transport === "stdio" ? draft.command.trim() : undefined,
        args:
          draft.transport === "stdio"
            ? draft.args
                .split("\n")
                .map((value) => value.trim())
                .filter(Boolean)
            : [],
        url: draft.transport === "streamable_http" ? draft.url.trim() : undefined,
        metadata,
        toolVisibility: {
          include: splitLines(draft.includeTools),
          exclude: splitLines(draft.excludeTools),
        },
        safety: {
          ...(editing?.safety ?? DEFAULT_AGENT_MCP_SAFETY),
          requiresApproval: draft.requiresApproval,
          allowSandboxed: draft.allowSandboxed,
          approvalTools: splitLines(draft.approvalTools),
        },
        ...(!editing ||
        Object.keys(secretBundle.env).length ||
        Object.keys(secretBundle.headers).length
          ? { secrets: secretBundle }
          : {}),
      };
      const saved = editing
        ? await updateAgentMcpServer({ ...input, id: editing.id })
        : await createAgentMcpServer(input);
      if (editing) {
        setHealth((current) => {
          const next = { ...current };
          delete next[editing.id];
          return next;
        });
        setDiagnostics((current) => ({ ...current, [editing.id]: saved.diagnostic ?? null }));
        setTestResults((current) => ({ ...current, [editing.id]: undefined }));
      }
      setServers((current) =>
        (editing
          ? current.map((server) => (server.id === editing.id ? saved : server))
          : [...current, saved]
        ).sort((a, b) => a.name.localeCompare(b.name)),
      );
      setDraft(EMPTY_DRAFT);
      setEditing(undefined);
      setAddOpen(false);
    } catch (createError) {
      setSaveError(messageFromError(createError));
    }
  }

  function splitLines(value: string) {
    return value
      .split("\n")
      .map((item) => item.trim())
      .filter(Boolean);
  }

  return (
    <section className="settings-group" aria-labelledby="mcp-servers-heading">
      <div className="settings-group-header">
        <h3 id="mcp-servers-heading" className="settings-group-heading">
          {t("settingsPanels.mcp.heading")}
        </h3>
        <button
          type="button"
          className="btn btn-secondary"
          disabled={!!busyId}
          onClick={openCreate}
        >
          <IconPlusMedium size={14} />
          {t("settingsPanels.mcp.addServer")}
        </button>
      </div>
      <p className="settings-group-description">{t("settingsPanels.mcp.description")}</p>
      {!servers.some((server) => server.url?.replace(/\/$/, "") === "https://mcp.vercel.com") ? (
        <div className="settings-group-description">
          <p>{t("settingsPanels.mcp.vercelPrerequisite")}</p>
          <button
            type="button"
            className="btn btn-secondary"
            disabled={loading || !!busyId}
            onClick={() => void addVercel()}
          >
            {t("settingsPanels.mcp.addVercel")}
          </button>
        </div>
      ) : null}

      {error ? (
        <InlineNotice
          tone="warning"
          body={error}
          actions={
            <button type="button" className="btn btn-secondary" onClick={() => void load()}>
              {t("common.tryAgain")}
            </button>
          }
        />
      ) : null}

      <div className="settings-card">
        <div className="settings-rows">
          {loading ? (
            <div className="settings-row">
              <p className="settings-row-description">{t("settingsPanels.mcp.loading")}</p>
            </div>
          ) : servers.length === 0 ? (
            <div className="settings-row">
              <div className="settings-row-info">
                <h4 className="settings-row-title">{t("settingsPanels.mcp.emptyTitle")}</h4>
                <p className="settings-row-description">{t("settingsPanels.mcp.emptyBody")}</p>
              </div>
            </div>
          ) : (
            servers.map((server) => {
              const connectCleanup = connectingId !== server.id && hasOauthConnectCleanup(server);
              const diagnostic =
                server.id in diagnostics ? diagnostics[server.id] : server.diagnostic;
              const observedHealth = health[server.id] ?? diagnostic?.state;
              const resultMessage = checkResultMessage(testResults[server.id], t);
              return (
                <div className="settings-row" key={server.id}>
                  <div className="settings-row-info">
                    <h4 className="settings-row-title">{server.name}</h4>
                    <p className="settings-row-description">
                      {server.transport === "stdio" ? server.command : server.url}
                      {server.metadata.needsReview === true
                        ? t("settingsPanels.mcp.needsReviewSuffix")
                        : ""}
                      {` · ${t("settingsPanels.mcp.managedOrigin")}`}
                      {server.metadata.oauthCleanupPending === true
                        ? ` · ${t("settingsPanels.mcp.localClearUnconfirmed")}`
                        : ""}
                      {typeof server.metadata.oauthRetiredSecretRef === "string"
                        ? ` · ${t("settingsPanels.mcp.previousCredentialPending")}`
                        : ""}
                      {server.metadata.preset === "vercel"
                        ? ` · ${t("settingsPanels.mcp.vercelPrerequisite")}`
                        : ""}
                      {observedHealth
                        ? ` · ${t(observedHealth === "connected" ? "settingsPanels.mcp.healthConnected" : observedHealth === "reconnect_required" ? "settingsPanels.mcp.healthReconnect" : "settingsPanels.mcp.healthUnavailable")}`
                        : ""}
                      {resultMessage || diagnostic?.message
                        ? ` · ${resultMessage || diagnostic?.message}`
                        : ""}
                    </p>
                    {hasOauthRevocationHistory(server) ? (
                      <p className="settings-row-description" role="note">
                        {t("settingsPanels.mcp.revocationHistoryWarning")}
                      </p>
                    ) : null}
                    {connectCleanup ? (
                      <p className="settings-row-description" role="note">
                        {t("settingsPanels.mcp.connectCleanupWarning")}
                      </p>
                    ) : null}
                    {diagnostic ? (
                      <p className="settings-row-description" role="status">
                        {diagnosticContext(diagnostic, t)}
                      </p>
                    ) : null}
                  </div>
                  <div className="settings-row-control">
                    {server.metadata.legacyAuth === "oauth" || server.metadata.auth === "oauth" ? (
                      connectingId === server.id ? (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={() => void cancelOauth(server)}
                        >
                          {t("common.cancel")}
                        </button>
                      ) : (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          disabled={!!busyId}
                          onClick={() => void connectOauth(server)}
                        >
                          {server.metadata.oauthConnected === true
                            ? t("settingsPanels.mcp.reconnect")
                            : t("settingsPanels.mcp.connect")}
                        </button>
                      )
                    ) : null}
                    {(server.metadata.legacyAuth === "oauth" || server.metadata.auth === "oauth") &&
                    (server.metadata.oauthConnected === true ||
                      server.metadata.oauthCleanupPending === true ||
                      connectCleanup ||
                      typeof server.metadata.oauthPendingSecretRef === "string" ||
                      typeof server.metadata.oauthRetiredSecretRef === "string") ? (
                      <button
                        type="button"
                        className="btn btn-secondary"
                        disabled={!!busyId}
                        onClick={() => void disconnectOauth(server)}
                      >
                        {t("settingsPanels.mcp.disconnect")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.mcp.configureName", { name: server.name })}
                      disabled={!!busyId}
                      onClick={() => openEdit(server)}
                    >
                      <IconSettingsGear4 size={14} />
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.mcp.testName", { name: server.name })}
                      disabled={!!busyId}
                      onClick={() => void test(server)}
                    >
                      <IconArrowRotateClockwise size={14} />
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.mcp.deleteName", { name: server.name })}
                      disabled={!!busyId}
                      onClick={() => setToDelete(server)}
                    >
                      <IconTrashCan size={14} />
                    </button>
                    <Switch
                      checked={server.enabled}
                      disabled={!!busyId}
                      aria-label={t("settingsPanels.mcp.enabledName", { name: server.name })}
                      onCheckedChange={(enabled) => void toggle(server, enabled)}
                    />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      <Dialog
        open={addOpen}
        onClose={() => {
          setAddOpen(false);
          setEditing(undefined);
        }}
        title={
          editing
            ? t("settingsPanels.mcp.configureName", { name: editing.name })
            : t("settingsPanels.mcp.addTitle")
        }
        description={
          editing ? t("settingsPanels.mcp.editDescription") : t("settingsPanels.mcp.addDescription")
        }
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={() => {
                setAddOpen(false);
                setEditing(undefined);
              }}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              disabled={!draft.name.trim()}
              onClick={() => void save()}
            >
              {editing ? t("settingsPanels.mcp.saveChanges") : t("settingsPanels.mcp.addServer")}
            </button>
          </>
        }
      >
        <div className="dialog-body">
          {saveError ? <InlineNotice tone="warning" body={saveError} /> : null}
          {editing?.metadata.legacyAuth === "oauth" ? (
            <InlineNotice tone="warning" body={t("settingsPanels.mcp.legacyOauth")} />
          ) : null}
          <label className="dialog-field">
            {t("settingsPanels.mcp.name")}
            <input
              className="dialog-input"
              value={draft.name}
              onChange={(event) =>
                setDraft((current) => ({ ...current, name: event.target.value }))
              }
            />
          </label>
          <label className="dialog-field">
            {t("settingsPanels.mcp.transport")}
            <select
              className="dialog-input"
              value={draft.transport}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  transport: event.target.value as AgentMcpTransport,
                }))
              }
            >
              <option value="stdio">{t("settingsPanels.mcp.transportStdio")}</option>
              <option value="streamable_http">Streamable HTTP</option>
            </select>
          </label>
          {draft.transport === "stdio" ? (
            <>
              <label className="dialog-field">
                {t("settingsPanels.mcp.command")}
                <input
                  className="dialog-input"
                  value={draft.command}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, command: event.target.value }))
                  }
                />
              </label>
              <label className="dialog-field">
                {t("settingsPanels.mcp.arguments")}
                <textarea
                  className="dialog-textarea"
                  value={draft.args}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, args: event.target.value }))
                  }
                />
              </label>
            </>
          ) : (
            <>
              <label className="dialog-field">
                URL
                <input
                  className="dialog-input"
                  value={draft.url}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, url: event.target.value }))
                  }
                />
              </label>
              <label className="dialog-checkbox">
                <input
                  type="checkbox"
                  checked={draft.oauth}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, oauth: event.target.checked }))
                  }
                />
                {t("settingsPanels.mcp.authenticateOauth")}
              </label>
            </>
          )}
          <label className="dialog-field">
            {t("settingsPanels.mcp.envVars")}
            <textarea
              className="dialog-textarea"
              placeholder={'{"TOKEN":"..."}'}
              value={draft.env}
              onChange={(event) => setDraft((current) => ({ ...current, env: event.target.value }))}
            />
          </label>
          <label className="dialog-field">
            {t("settingsPanels.mcp.httpHeaders")}
            <textarea
              className="dialog-textarea"
              placeholder={'{"Authorization":"Bearer ..."}'}
              value={draft.headers}
              onChange={(event) =>
                setDraft((current) => ({ ...current, headers: event.target.value }))
              }
            />
          </label>
          <label className="dialog-field">
            {t("settingsPanels.mcp.allowedTools")}
            <textarea
              className="dialog-textarea"
              placeholder={t("settingsPanels.mcp.allowedToolsPlaceholder")}
              value={draft.includeTools}
              onChange={(event) =>
                setDraft((current) => ({ ...current, includeTools: event.target.value }))
              }
            />
          </label>
          <label className="dialog-field">
            {t("settingsPanels.mcp.blockedTools")}
            <textarea
              className="dialog-textarea"
              value={draft.excludeTools}
              onChange={(event) =>
                setDraft((current) => ({ ...current, excludeTools: event.target.value }))
              }
            />
          </label>
          <label className="dialog-field">
            {t("settingsPanels.mcp.approvalTools")}
            <textarea
              className="dialog-textarea"
              value={draft.approvalTools}
              disabled={draft.requiresApproval}
              onChange={(event) =>
                setDraft((current) => ({ ...current, approvalTools: event.target.value }))
              }
            />
          </label>
          <div className="settings-card">
            <div className="settings-rows">
              <div className="settings-row">
                <span className="settings-row-title">
                  {t("settingsPanels.mcp.requireApprovalAll")}
                </span>
                <Switch
                  checked={draft.requiresApproval}
                  aria-label={t("settingsPanels.mcp.requireApprovalAll")}
                  onCheckedChange={(value) =>
                    setDraft((current) => ({ ...current, requiresApproval: value }))
                  }
                />
              </div>
              <div className="settings-row">
                <span className="settings-row-title">
                  {draft.transport === "stdio"
                    ? t("settingsPanels.mcp.allowSandboxedMacos")
                    : t("settingsPanels.mcp.allowSandboxed")}
                </span>
                <Switch
                  checked={draft.allowSandboxed}
                  aria-label={t("settingsPanels.mcp.allowSandboxed")}
                  onCheckedChange={(value) =>
                    setDraft((current) => ({ ...current, allowSandboxed: value }))
                  }
                />
              </div>
            </div>
          </div>
        </div>
      </Dialog>
      <ConfirmDialog
        open={Boolean(toDelete)}
        onClose={() => setToDelete(undefined)}
        onConfirm={() => (toDelete ? remove(toDelete) : undefined)}
        title={
          toDelete
            ? t("settingsPanels.mcp.deleteTitleName", { name: toDelete.name })
            : t("settingsPanels.mcp.deleteTitle")
        }
        description={t("settingsPanels.mcp.deleteDescription")}
        confirmLabel={t("settingsPanels.mcp.deleteServer")}
        confirmBusyLabel={t("settingsPanels.mcp.deleting")}
        destructive
      />
    </section>
  );
}
