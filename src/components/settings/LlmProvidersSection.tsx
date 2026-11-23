import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconSettingsGear4 } from "central-icons/IconSettingsGear4";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { useCallback, useEffect, useRef, useState } from "react";
import { type TFunction, useT } from "../../i18n";
import { errorCode, messageFromError, rawMessageFromError } from "../../lib/errors";
import {
  type LlmCliStatusDto,
  type LlmEndpointDto,
  type LlmProviderRef,
  type LlmProviderTestResultDto,
  type LlmProvidersDto,
  type LlmUsage,
  type StructuredOutputLevel,
  isActivityEligible,
  llmDeleteEndpoint,
  llmDetectClis,
  llmProviders,
  llmSaveEndpoint,
  llmSetUsage,
  llmTestProvider,
  openAntigravityLogin,
  parseProviderRefKey,
  providerRefKey,
} from "../../lib/llm-providers";
import { isLoopbackUrl } from "../../lib/local-generation";
import { PROVIDER_MODEL_SETTINGS_CHANGED_EVENT } from "../../lib/model-privacy";
import { probeLocalGenerationEndpoint } from "../../lib/tauri";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Dialog } from "../ui/Dialog";
import { InlineNotice } from "../ui/InlineNotice";
import { RuntimeLogo } from "../brand/RuntimeLogo";

export type LlmProvidersSectionProps = {
  onChatProviderChanged?: () => void;
  onProvidersChanged?: (providers: LlmProvidersDto) => void;
};

type EndpointDraft = {
  id?: string;
  name: string;
  baseUrl: string;
  modelId: string;
  apiKey: string;
  clearApiKey: boolean;
};

const EMPTY_ENDPOINT_DRAFT: EndpointDraft = {
  name: "",
  baseUrl: "",
  modelId: "",
  apiKey: "",
  clearApiKey: false,
};

function formatLevel(level: StructuredOutputLevel | undefined, t: TFunction): string {
  switch (level) {
    case "none":
      return t("settingsPanels.llm.levelNone");
    case "prompt":
      return t("settingsPanels.llm.levelPrompt");
    case "json_object":
      return t("settingsPanels.llm.levelJsonObject");
    case "json_schema":
      return t("settingsPanels.llm.levelJsonSchema");
    case "strict":
      return t("settingsPanels.llm.levelStrict");
    default:
      return t("settingsPanels.llm.unknownLevel");
  }
}

export function LlmProvidersSection({
  onChatProviderChanged,
  onProvidersChanged,
}: LlmProvidersSectionProps) {
  const t = useT();

  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string>();
  const [data, setData] = useState<LlmProvidersDto | null>(null);
  const [clis, setClis] = useState<LlmCliStatusDto[]>([]);
  useEffect(() => {
    if (data) onProvidersChanged?.(data);
  }, [data, onProvidersChanged]);
  const [clisDetecting, setClisDetecting] = useState(false);

  // Per-usage errors (e.g. rejection when structured output is insufficient)
  const [usageErrors, setUsageErrors] = useState<Partial<Record<LlmUsage, string>>>({});
  const [busyUsage, setBusyUsage] = useState<Partial<Record<LlmUsage, boolean>>>({});

  // Testing feedback
  const [cliTestResults, setCliTestResults] = useState<Record<string, string>>({});
  const [endpointTestResults, setEndpointTestResults] = useState<Record<string, string>>({});
  const [testingTarget, setTestingTarget] = useState<string>();
  const [agyLoginOpening, setAgyLoginOpening] = useState(false);
  const [agyLoginError, setAgyLoginError] = useState<string>();
  const agyLoginPending = useRef(false);

  // Add / Edit Endpoint dialog state
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingEndpoint, setEditingEndpoint] = useState<LlmEndpointDto | null>(null);
  const [draft, setDraft] = useState<EndpointDraft>(EMPTY_ENDPOINT_DRAFT);
  const [dialogError, setDialogError] = useState<string>();
  const [dialogSaving, setDialogSaving] = useState(false);
  const [probedModels, setProbedModels] = useState<string[]>([]);
  const [probingModels, setProbingModels] = useState(false);

  // Delete ConfirmDialog state
  const [deletingEndpoint, setDeletingEndpoint] = useState<LlmEndpointDto | null>(null);

  const loadAll = useCallback(async () => {
    setLoading(true);
    setLoadError(undefined);
    try {
      const [providersDto, clisDto] = await Promise.all([llmProviders(), llmDetectClis()]);
      setData(providersDto);
      setClis(clisDto);
    } catch (err) {
      setLoadError(messageFromError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  // Re-read note provider preferences when model settings change elsewhere.
  // Text-model selection never changes the chat engine.
  useEffect(() => {
    const refresh = () => {
      void llmProviders()
        .then(setData)
        .catch(() => undefined);
    };
    window.addEventListener(PROVIDER_MODEL_SETTINGS_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(PROVIDER_MODEL_SETTINGS_CHANGED_EVENT, refresh);
  }, []);

  const handleRefreshClis = async () => {
    setClisDetecting(true);
    try {
      const detected = await llmDetectClis();
      setClis(detected);
    } catch (err) {
      setLoadError(messageFromError(err));
    } finally {
      setClisDetecting(false);
    }
  };

  const handleUsageChange = async (usage: LlmUsage, rawKey: string) => {
    const providerRef = parseProviderRefKey(rawKey);
    setUsageErrors((prev) => ({ ...prev, [usage]: undefined }));
    setBusyUsage((prev) => ({ ...prev, [usage]: true }));

    try {
      if (
        usage === "activity" &&
        providerRef.kind === "cli" &&
        data?.cliLevels[providerRef.id] === undefined &&
        clis.some((cli) => cli.id === providerRef.id && cli.installed && cli.oneShot)
      ) {
        setTestingTarget(`cli:${providerRef.id}`);
        let result: LlmProviderTestResultDto;
        try {
          result = await llmTestProvider(providerRef);
        } catch (err) {
          // CLI failures have one native code. Preserve the actual cause and
          // offer the managed-profile action only for Antigravity login errors.
          const detail = rawMessageFromError(err);
          const cliFailure = errorCode(err) === "llm_cli_failed";
          const message =
            cliFailure && /quota|rate[ -]?limit|resource.exhausted|usage limit/i.test(detail)
              ? t("lib.errors.llm.usageLimit")
              : cliFailure &&
                  providerRef.id === "agy" &&
                  /not authenticated|unauthenticated|unauthorized|\b401\b|not logged in|login required|authentication (?:required|failed)|please (?:log ?in|authenticate)|run .{0,30}agy login/i.test(
                    detail,
                  )
                ? t("lib.errors.llm.agyAuthenticationRequired")
                : messageFromError(err);
          setUsageErrors((prev) => ({ ...prev, [usage]: message }));
          return;
        } finally {
          setTestingTarget(undefined);
        }
        setCliTestResults((prev) => ({
          ...prev,
          [providerRef.id]: t("settingsPanels.llm.testSuccess", {
            ms: result.latencyMs,
            level: formatLevel(result.structuredOutput, t),
          }),
        }));
        setData((prev) =>
          prev
            ? {
                ...prev,
                cliLevels: { ...prev.cliLevels, [providerRef.id]: result.structuredOutput },
              }
            : prev,
        );
        if (!isActivityEligible(result.structuredOutput)) {
          setUsageErrors((prev) => ({ ...prev, [usage]: t("lib.errors.llm.structuredOutput") }));
          return;
        }
      }
      const updated = await llmSetUsage(usage, providerRef);
      setData(updated);
      if (usage === "chat") {
        onChatProviderChanged?.();
      }
    } catch (err) {
      setUsageErrors((prev) => ({ ...prev, [usage]: messageFromError(err) }));
    } finally {
      setBusyUsage((prev) => ({ ...prev, [usage]: false }));
    }
  };

  const handleTestCli = async (cli: LlmCliStatusDto) => {
    const targetKey = `cli:${cli.id}`;
    setTestingTarget(targetKey);
    try {
      const result = await llmTestProvider({ kind: "cli", id: cli.id });
      setCliTestResults((prev) => ({
        ...prev,
        [cli.id]: t("settingsPanels.llm.testSuccess", {
          ms: result.latencyMs,
          level: formatLevel(result.structuredOutput, t),
        }),
      }));
      // Update cliLevels in state if present
      setData((prev) =>
        prev
          ? {
              ...prev,
              cliLevels: { ...prev.cliLevels, [cli.id]: result.structuredOutput },
            }
          : prev,
      );
    } catch (err) {
      setCliTestResults((prev) => ({
        ...prev,
        [cli.id]: messageFromError(err),
      }));
    } finally {
      setTestingTarget(undefined);
    }
  };

  const handleAntigravityLogin = async () => {
    if (agyLoginPending.current) return;
    agyLoginPending.current = true;
    setAgyLoginOpening(true);
    setAgyLoginError(undefined);
    try {
      await openAntigravityLogin();
    } catch (error) {
      setAgyLoginError(messageFromError(error));
    } finally {
      agyLoginPending.current = false;
      setAgyLoginOpening(false);
    }
  };

  const handleTestEndpoint = async (endpoint: LlmEndpointDto) => {
    const targetKey = `endpoint:${endpoint.id}`;
    setTestingTarget(targetKey);
    try {
      const result = await llmTestProvider({ kind: "endpoint", id: endpoint.id });
      setEndpointTestResults((prev) => ({
        ...prev,
        [endpoint.id]: t("settingsPanels.llm.testSuccess", {
          ms: result.latencyMs,
          level: formatLevel(result.structuredOutput, t),
        }),
      }));
      // Refresh providers so endpoint.structuredOutput is stored
      const refreshed = await llmProviders();
      setData(refreshed);
    } catch (err) {
      setEndpointTestResults((prev) => ({
        ...prev,
        [endpoint.id]: messageFromError(err),
      }));
    } finally {
      setTestingTarget(undefined);
    }
  };

  const openAddDialog = () => {
    setEditingEndpoint(null);
    setDraft(EMPTY_ENDPOINT_DRAFT);
    setDialogError(undefined);
    setProbedModels([]);
    setDialogOpen(true);
  };

  const openEditDialog = (endpoint: LlmEndpointDto) => {
    setEditingEndpoint(endpoint);
    setDraft({
      id: endpoint.id,
      name: endpoint.name,
      baseUrl: endpoint.baseUrl,
      modelId: endpoint.modelId,
      apiKey: "",
      clearApiKey: false,
    });
    setDialogError(undefined);
    setProbedModels([]);
    setDialogOpen(true);
  };

  const handleProbeModels = async () => {
    if (!draft.baseUrl.trim()) return;
    setProbingModels(true);
    setDialogError(undefined);
    try {
      const result = await probeLocalGenerationEndpoint({
        baseUrl: draft.baseUrl.trim(),
        apiKey: draft.apiKey.trim(),
        // Without a typed key, the backend uses the key saved for the
        // endpoint being edited (unless the user is removing it).
        endpointId: editingEndpoint && !draft.clearApiKey ? editingEndpoint.id : undefined,
      });
      setProbedModels(result.models);
    } catch (err) {
      setDialogError(messageFromError(err));
    } finally {
      setProbingModels(false);
    }
  };

  const handleSaveEndpointSubmit = async () => {
    if (!draft.name.trim() || !draft.baseUrl.trim() || !draft.modelId.trim()) {
      return;
    }
    setDialogSaving(true);
    setDialogError(undefined);
    try {
      const updated = await llmSaveEndpoint({
        id: editingEndpoint?.id,
        name: draft.name.trim(),
        baseUrl: draft.baseUrl.trim(),
        modelId: draft.modelId.trim(),
        apiKey: draft.apiKey ? draft.apiKey : undefined,
        clearApiKey: editingEndpoint ? draft.clearApiKey : undefined,
      });
      setData(updated);
      setDialogOpen(false);
      // If chat uses this endpoint, notify
      if (
        editingEndpoint &&
        updated.usage.chat.kind === "endpoint" &&
        updated.usage.chat.id === editingEndpoint.id
      ) {
        onChatProviderChanged?.();
      }
    } catch (err) {
      setDialogError(messageFromError(err));
    } finally {
      setDialogSaving(false);
    }
  };

  const handleDeleteEndpointConfirm = async () => {
    if (!deletingEndpoint) return;
    try {
      const updated = await llmDeleteEndpoint(deletingEndpoint.id);
      setData(updated);
      if (data?.usage.chat.kind === "endpoint" && data.usage.chat.id === deletingEndpoint.id) {
        onChatProviderChanged?.();
      }
    } catch (err) {
      setLoadError(messageFromError(err));
    } finally {
      setDeletingEndpoint(null);
    }
  };

  const isRemoteDraft = draft.baseUrl.trim().length > 0 && !isLoopbackUrl(draft.baseUrl.trim());

  if (loading) {
    return (
      <section className="settings-group" aria-labelledby="llm-providers-heading">
        <div className="settings-group-header">
          <h3 id="llm-providers-heading" className="settings-group-heading">
            {t("settingsPanels.llm.heading")}
          </h3>
        </div>
        <p className="settings-group-description">{t("settingsPanels.llm.loading")}</p>
      </section>
    );
  }

  const endpoints = data?.endpoints ?? [];
  const usage = data?.usage ?? {
    chat: { kind: "cli", id: "claude" },
    notes: { kind: "openrouter" },
    activity: { kind: "none" },
  };

  // Provider options generator
  const getOptionsForUsage = (forUsage: LlmUsage) => {
    const options: { value: string; label: string; disabled?: boolean }[] = [];

    if (forUsage === "activity") {
      options.push({ value: "none", label: t("settingsPanels.llm.providerNone") });
    } else if (forUsage !== "chat") {
      options.push({ value: "openrouter", label: t("settingsPanels.llm.providerOpenRouter") });
    }

    for (const ep of forUsage === "chat" ? [] : endpoints) {
      options.push({ value: providerRefKey({ kind: "endpoint", id: ep.id }), label: ep.name });
    }

    for (const cli of clis) {
      // Notes and activity send content to the CLI, so only CLIs that run
      // one-shot calls with every tool off or in their least-privileged mode
      // (codex, agy) can take them.
      const toolsOn = forUsage !== "chat" && !cli.oneShot;
      const disabled = !cli.installed || toolsOn;
      const suffix = !cli.installed
        ? t("settingsPanels.llm.notInstalledSuffix")
        : toolsOn
          ? t("settingsPanels.llm.toolsOnSuffix")
          : "";
      options.push({
        value: providerRefKey({ kind: "cli", id: cli.id }),
        label: `${cli.name}${suffix}`,
        disabled,
      });
    }

    return options;
  };

  return (
    <section className="settings-group" aria-labelledby="llm-providers-heading">
      <div className="settings-group-header">
        <h3 id="llm-providers-heading" className="settings-group-heading">
          {t("settingsPanels.llm.heading")}
        </h3>
      </div>
      <p className="settings-group-description">{t("settingsPanels.llm.description")}</p>

      {loadError ? (
        <InlineNotice
          tone="warning"
          body={loadError}
          actions={
            <button type="button" className="btn btn-secondary" onClick={() => void loadAll()}>
              {t("common.tryAgain")}
            </button>
          }
        />
      ) : null}
      {/* Block 1: Providers for each use */}
      <div className="settings-group-header">
        <h4 className="settings-group-heading">{t("settingsPanels.llm.usageHeading")}</h4>
      </div>
      <p className="settings-group-description">{t("settingsPanels.llm.usageDescription")}</p>
      <div className="settings-card">
        <div className="settings-rows">
          {/* Chat row */}
          <div className="settings-row">
            <div className="settings-row-info">
              <h4 className="settings-row-title">{t("settingsPanels.llm.usageChat")}</h4>
              {usageErrors.chat ? (
                <p className="settings-row-error" role="alert">
                  {usageErrors.chat}
                </p>
              ) : null}
            </div>
            <div className="settings-row-control">
              <UsageProviderMark provider={usage.chat} endpoints={endpoints} />
              <select
                className="dialog-input"
                aria-label={t("settingsPanels.llm.usageChat")}
                value={providerRefKey(usage.chat)}
                disabled={busyUsage.chat}
                onChange={(e) => void handleUsageChange("chat", e.target.value)}
              >
                {getOptionsForUsage("chat").map((opt) => (
                  <option key={opt.value} value={opt.value} disabled={opt.disabled}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* Notes row */}
          <div className="settings-row">
            <div className="settings-row-info">
              <h4 className="settings-row-title">{t("settingsPanels.llm.usageNotes")}</h4>
              {usageErrors.notes ? (
                <p className="settings-row-error" role="alert">
                  {usageErrors.notes}
                </p>
              ) : null}
            </div>
            <div className="settings-row-control">
              <UsageProviderMark provider={usage.notes} endpoints={endpoints} />
              <select
                className="dialog-input"
                aria-label={t("settingsPanels.llm.usageNotes")}
                value={providerRefKey(usage.notes)}
                disabled={busyUsage.notes}
                onChange={(e) => void handleUsageChange("notes", e.target.value)}
              >
                {getOptionsForUsage("notes").map((opt) => (
                  <option key={opt.value} value={opt.value} disabled={opt.disabled}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* Dictation cleanup is not here: it runs only on an OpenRouter
              model, chosen in the dictation style settings. */}

          {/* Activity row */}
          <div className="settings-row">
            <div className="settings-row-info">
              <h4 className="settings-row-title">{t("settingsPanels.llm.usageActivity")}</h4>
              <p className="settings-row-description">
                {t("settingsPanels.llm.usageActivityDescription")}
              </p>
              {busyUsage.activity && testingTarget ? (
                <p className="settings-row-description" role="status">
                  {t("settingsPanels.llm.testing")}
                </p>
              ) : null}
              {usageErrors.activity ? (
                <p className="settings-row-error" role="alert">
                  {usageErrors.activity}
                </p>
              ) : null}
            </div>
            <div className="settings-row-control">
              <UsageProviderMark provider={usage.activity} endpoints={endpoints} />
              <select
                className="dialog-input"
                aria-label={t("settingsPanels.llm.usageActivity")}
                value={providerRefKey(usage.activity)}
                disabled={busyUsage.activity || testingTarget !== undefined}
                onChange={(e) => void handleUsageChange("activity", e.target.value)}
              >
                {getOptionsForUsage("activity").map((opt) => (
                  <option key={opt.value} value={opt.value} disabled={opt.disabled}>
                    {opt.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </div>
      </div>

      {/* Block 2: Agent CLIs */}
      <div className="settings-group-header" style={{ marginTop: "var(--sp-6)" }}>
        <h4 className="settings-group-heading">{t("settingsPanels.llm.clisHeading")}</h4>
        <button
          type="button"
          className="btn btn-secondary"
          disabled={clisDetecting}
          onClick={() => void handleRefreshClis()}
        >
          <IconArrowRotateClockwise size={14} />
          {t("settingsPanels.llm.refreshClis")}
        </button>
      </div>
      <p className="settings-group-description">{t("settingsPanels.llm.clisDescription")}</p>

      <div className="settings-card">
        <div className="settings-rows">
          {clis.map((cli) => {
            const isInstalled = cli.installed;
            const details = isInstalled
              ? [cli.path, cli.version].filter(Boolean).join(" · ")
              : t("settingsPanels.llm.notInstalledReason");
            const testResult = cliTestResults[cli.id];
            const isTesting = testingTarget === `cli:${cli.id}`;

            return (
              <div className="settings-row" key={cli.id}>
                <div className="settings-row-info">
                  <h4 className="settings-row-title settings-row-title-with-mark">
                    <RuntimeLogo id={cli.id} name={cli.name} size={16} decorative />
                    {cli.name}
                  </h4>
                  <p className="settings-row-description">
                    {details}
                    {testResult ? ` · ${testResult}` : ""}
                  </p>
                  {cli.id === "agy" && isInstalled ? (
                    <>
                      <p className="settings-row-description">
                        {t("settingsPanels.llm.agyLoginDescription")}
                      </p>
                      {agyLoginError ? (
                        <InlineNotice
                          tone="destructive"
                          role="alert"
                          body={t("settingsPanels.llm.agyLoginError", { detail: agyLoginError })}
                        />
                      ) : null}
                    </>
                  ) : null}
                  {cli.oneShot ? null : (
                    <p className="settings-row-description">
                      {t("settingsPanels.llm.toolsOnReason")}
                    </p>
                  )}
                </div>
                <div className="settings-row-control">
                  {cli.id === "agy" && isInstalled ? (
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={agyLoginOpening || isTesting}
                      onClick={() => void handleAntigravityLogin()}
                    >
                      {t(
                        agyLoginOpening
                          ? "settingsPanels.llm.agyLoginOpening"
                          : "settingsPanels.llm.agyLogin",
                      )}
                    </button>
                  ) : null}
                  {isInstalled && cli.oneShot ? (
                    <button
                      type="button"
                      className="btn btn-secondary"
                      disabled={
                        testingTarget !== undefined ||
                        busyUsage.activity ||
                        (cli.id === "agy" && agyLoginOpening)
                      }
                      onClick={() => void handleTestCli(cli)}
                    >
                      <IconArrowRotateClockwise size={14} />
                      {isTesting ? t("settingsPanels.llm.testing") : t("settingsPanels.llm.test")}
                    </button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Block 3: Endpoints */}
      <div className="settings-group-header" style={{ marginTop: "var(--sp-6)" }}>
        <h4 className="settings-group-heading">{t("settingsPanels.llm.endpointsHeading")}</h4>
        <button type="button" className="btn btn-secondary" onClick={openAddDialog}>
          <IconPlusMedium size={14} />
          {t("settingsPanels.llm.addEndpoint")}
        </button>
      </div>
      <p className="settings-group-description">{t("settingsPanels.llm.endpointsDescription")}</p>

      <div className="settings-card">
        <div className="settings-rows">
          {endpoints.length === 0 ? (
            <div className="settings-row">
              <p className="settings-row-description">{t("settingsPanels.llm.noEndpoints")}</p>
            </div>
          ) : (
            endpoints.map((endpoint) => {
              const testResult = endpointTestResults[endpoint.id];

              const badges: string[] = [`${endpoint.baseUrl} (${endpoint.modelId})`];
              if (endpoint.hasApiKey) {
                badges.push(t("settingsPanels.llm.apiKeySaved"));
              }
              // A fresh test result already states the measured level.
              if (testResult) {
                badges.push(testResult);
              } else if (endpoint.structuredOutput) {
                badges.push(
                  t("settingsPanels.llm.structuredOutputPrefix", {
                    level: formatLevel(endpoint.structuredOutput, t),
                  }),
                );
              }

              return (
                <div className="settings-row" key={endpoint.id}>
                  <div className="settings-row-info">
                    <h4 className="settings-row-title settings-row-title-with-mark">
                      <RuntimeLogo id={endpoint.id} name={endpoint.name} size={16} decorative />
                      {endpoint.name}
                    </h4>
                    <p className="settings-row-description">{badges.join(" · ")}</p>
                  </div>
                  <div className="settings-row-control">
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.llm.testEndpoint", { name: endpoint.name })}
                      disabled={testingTarget !== undefined || busyUsage.activity}
                      onClick={() => void handleTestEndpoint(endpoint)}
                    >
                      <IconArrowRotateClockwise size={14} />
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.llm.configureEndpoint", {
                        name: endpoint.name,
                      })}
                      onClick={() => openEditDialog(endpoint)}
                    >
                      <IconSettingsGear4 size={14} />
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t("settingsPanels.llm.deleteEndpoint", { name: endpoint.name })}
                      onClick={() => setDeletingEndpoint(endpoint)}
                    >
                      <IconTrashCan size={14} />
                    </button>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* Add / Edit Dialog */}
      <Dialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        title={
          editingEndpoint
            ? t("settingsPanels.llm.editDialogTitle")
            : t("settingsPanels.llm.addDialogTitle")
        }
        footer={
          <>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => setDialogOpen(false)}
            >
              {t("settingsPanels.llm.cancel")}
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={
                dialogSaving || !draft.name.trim() || !draft.baseUrl.trim() || !draft.modelId.trim()
              }
              onClick={() => void handleSaveEndpointSubmit()}
            >
              {dialogSaving
                ? t("common.saving")
                : editingEndpoint
                  ? t("settingsPanels.llm.saveChanges")
                  : t("settingsPanels.llm.saveEndpoint")}
            </button>
          </>
        }
      >
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
          {dialogError ? (
            <p className="settings-row-error" role="alert">
              {dialogError}
            </p>
          ) : null}

          <label className="dialog-field">
            <span>{t("settingsPanels.llm.endpointName")}</span>
            <input
              className="dialog-input"
              value={draft.name}
              onChange={(e) => setDraft((prev) => ({ ...prev, name: e.target.value }))}
            />
          </label>

          <label className="dialog-field">
            <span>{t("settingsPanels.llm.endpointBaseUrl")}</span>
            <input
              className="dialog-input"
              value={draft.baseUrl}
              placeholder="http://localhost:11434/v1"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              onChange={(e) => setDraft((prev) => ({ ...prev, baseUrl: e.target.value }))}
            />
          </label>

          {isRemoteDraft ? (
            <p className="settings-local-model-warning" role="note">
              {t("settingsPanels.llm.remoteWarning")}
            </p>
          ) : null}

          <div className="dialog-field">
            <label htmlFor="llm-endpoint-model-id">{t("settingsPanels.llm.endpointModelId")}</label>
            <div style={{ display: "flex", gap: "var(--sp-2)", marginTop: "var(--sp-1)" }}>
              <input
                id="llm-endpoint-model-id"
                className="dialog-input"
                style={{ flex: 1 }}
                value={draft.modelId}
                list="llm-endpoint-models-datalist"
                placeholder="llama3.1:8b"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                onChange={(e) => setDraft((prev) => ({ ...prev, modelId: e.target.value }))}
              />
              <button
                type="button"
                className="btn btn-secondary"
                disabled={probingModels || !draft.baseUrl.trim()}
                onClick={() => void handleProbeModels()}
              >
                {probingModels
                  ? t("settingsPanels.llm.loadingModels")
                  : t("settingsPanels.llm.loadModels")}
              </button>
            </div>
            <datalist id="llm-endpoint-models-datalist">
              {probedModels.map((id) => (
                <option key={id} value={id} />
              ))}
            </datalist>
          </div>

          <label className="dialog-field">
            <span>{t("settingsPanels.llm.endpointApiKey")}</span>
            <input
              type="password"
              className="dialog-input"
              value={draft.apiKey}
              placeholder={
                editingEndpoint
                  ? t("settingsPanels.llm.editApiKeyHint")
                  : t("settingsPanels.llm.endpointApiKeyOptional")
              }
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              onChange={(e) => setDraft((prev) => ({ ...prev, apiKey: e.target.value }))}
            />
          </label>

          {editingEndpoint?.hasApiKey ? (
            <label style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
              <input
                type="checkbox"
                checked={draft.clearApiKey}
                onChange={(e) => setDraft((prev) => ({ ...prev, clearApiKey: e.target.checked }))}
              />
              <span>{t("settingsPanels.llm.clearApiKey")}</span>
            </label>
          ) : null}
        </div>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <ConfirmDialog
        open={Boolean(deletingEndpoint)}
        onClose={() => setDeletingEndpoint(null)}
        onConfirm={handleDeleteEndpointConfirm}
        destructive
        title={t("settingsPanels.llm.deleteTitle", {
          name: deletingEndpoint?.name ?? "",
        })}
        description={t("settingsPanels.llm.deleteDescription")}
        confirmLabel={t("settingsPanels.llm.deleteConfirm")}
      />
    </section>
  );
}

/** Native selects draw no icons, so the mark of the chosen CLI or endpoint
 * sits beside the select. */
function UsageProviderMark({
  provider,
  endpoints,
}: {
  provider: LlmProviderRef;
  endpoints: ReadonlyArray<{ id: string; name: string }>;
}) {
  if (provider.kind === "cli") return <RuntimeLogo id={provider.id} size={16} decorative />;
  if (provider.kind === "endpoint") {
    const name = endpoints.find((endpoint) => endpoint.id === provider.id)?.name ?? provider.id;
    return <RuntimeLogo id={provider.id} name={name} size={16} decorative />;
  }
  return null;
}
