import { listen } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { Fragment } from "react";
import { ConnectorProviderIcon } from "../connectors/ConnectorProviderIcon";
import { BrowserUseControl } from "../plugins/BrowserUseControl";
import { useExperimentalFlags } from "../../lib/experimental-flags";
import { ComputerUseControl } from "../plugins/ComputerUseControl";
import {
  accountStatusMeta,
  bundleMeta,
  bundlesForProvider,
  bundlesFromScopes,
  defaultBundlesForProvider,
  grantedFeatureLabels,
  isConnectorNotConfiguredError,
  scopesCoverBundles,
} from "../../lib/connectors";
import { useConnectorPolicy } from "../../lib/connector-policy";
import { errorCode, messageFromError } from "../../lib/errors";
import { type MessageKey, t as translate, useT } from "../../i18n";
import {
  CONNECTORS_CHANGED_EVENT,
  GITHUB_DEVICE_CODE_EVENT,
  connectorsCancelConnect,
  connectorsConnect,
  connectorsDisconnect,
  connectorsGithubCliConnect,
  connectorsGithubCliDetect,
  connectorsList,
  notionConnectorConnect,
  notionConnectorDisconnect,
  obsidianConfigure,
  obsidianDisconnect,
  obsidianStatus,
  type ConnectorAccount,
  type ConnectorProvider,
  type ConnectorScopeBundle,
  type GitHubDeviceCodePayload,
  type ObsidianStatus,
} from "../../lib/tauri";
import { CopyStateIcon } from "../ui/CopyStateIcon";
import { Checkbox } from "../ui/Checkbox";
import { Dialog } from "../ui/Dialog";
import { InlineNotice } from "../ui/InlineNotice";
import { HoverTip } from "../ui/HoverTip";
import { toast } from "../ui/Toaster";
import { SettingsPageHeader } from "./AppSettings";
import { IconCircleInfo } from "central-icons/IconCircleInfo";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";

type OAuthConnectorProvider = Extract<ConnectorProvider, "google" | "linear" | "github">;

const PROVIDER_NAMES = {
  google: "Google",
  linear: "Linear",
  notion: "Notion",
  github: "GitHub",
} satisfies Record<ConnectorProvider, string>;

/** One-line capability blurb shown while a provider is not connected: what
 * connecting it lets Clovy do, in the provider directory row. */
const PROVIDER_BLURB_KEYS = {
  google: "settingsPanels.connectors.blurbGoogle",
  linear: "settingsPanels.connectors.blurbLinear",
  github: "settingsPanels.connectors.blurbGithub",
} as const satisfies Record<OAuthConnectorProvider, MessageKey>;

const LINEAR_FULL_ACCESS_BUNDLES = [
  "linear_read",
  "linear_write",
] as const satisfies readonly ConnectorScopeBundle[];

type NotionConnectorRowProps = {
  account: ConnectorAccount | null;
  connecting: boolean;
  disconnecting: boolean;
  onDisconnect: () => void;
  onOpenConnectDialog: () => void;
  onOpenReconnectDialog: () => void;
};

type NotionConnectorState = "disconnected" | "connected" | "reconnect_required" | "unavailable";

type NotionConnectorActionsProps = Pick<
  NotionConnectorRowProps,
  "connecting" | "disconnecting" | "onDisconnect" | "onOpenConnectDialog" | "onOpenReconnectDialog"
> & {
  state: NotionConnectorState;
};

function notionConnectorState(account: ConnectorAccount | null): NotionConnectorState {
  if (account?.status === "unavailable") return "unavailable";
  if (account?.status === "reconnect_required") return "reconnect_required";
  if (account?.status === "connected") return "connected";
  return "disconnected";
}

function NotionConnectorActions({
  state,
  connecting,
  disconnecting,
  onDisconnect,
  onOpenConnectDialog,
  onOpenReconnectDialog,
}: NotionConnectorActionsProps) {
  const t = useT();
  const busy = connecting || disconnecting;
  const disconnectButton = (
    <button
      type="button"
      className="btn btn-ghost"
      aria-label={t("settingsPanels.connectors.disconnectNotion")}
      disabled={busy}
      aria-busy={disconnecting || undefined}
      onClick={onDisconnect}
    >
      {disconnecting
        ? t("settingsPanels.connectors.disconnecting")
        : t("settingsPanels.connectors.disconnect")}
    </button>
  );

  if (state === "connected" || state === "unavailable") return disconnectButton;
  if (state === "reconnect_required") {
    return (
      <>
        <button
          type="button"
          className="btn btn-secondary"
          aria-label={t("settingsPanels.connectors.reconnectNotion")}
          disabled={busy}
          aria-busy={connecting || undefined}
          onClick={onOpenReconnectDialog}
        >
          {connecting
            ? t("settingsPanels.connectors.waitingForBrowser")
            : t("settingsPanels.connectors.reconnect")}
        </button>
        {disconnectButton}
      </>
    );
  }
  return (
    <button
      type="button"
      className="btn btn-secondary"
      aria-label={t("settingsPanels.connectors.connectNotion")}
      disabled={busy}
      aria-busy={connecting || undefined}
      onClick={onOpenConnectDialog}
    >
      {connecting
        ? t("settingsPanels.connectors.waitingForBrowser")
        : t("settingsPanels.connectors.connect")}
    </button>
  );
}

function NotionConnectorRow({
  account,
  connecting,
  disconnecting,
  onDisconnect,
  onOpenConnectDialog,
  onOpenReconnectDialog,
}: NotionConnectorRowProps) {
  const t = useT();
  const state = notionConnectorState(account);
  const details = {
    disconnected: {
      subtitle: t("settingsPanels.connectors.notionBlurb"),
      statusLabel: null,
      statusTone: "warning",
    },
    connected: {
      subtitle: t("settingsPanels.connectors.notionConnectedBlurb"),
      statusLabel: t("settingsPanels.connectors.connected"),
      statusTone: "ok",
    },
    reconnect_required: {
      subtitle: t("settingsPanels.connectors.notionReconnectBlurb"),
      statusLabel: t("settingsPanels.connectors.reconnectNeeded"),
      statusTone: "warning",
    },
    unavailable: {
      subtitle: t("settingsPanels.connectors.notionUnavailableBlurb"),
      statusLabel: t("settingsPanels.connectors.statusUnavailable"),
      statusTone: "warning",
    },
  } as const;
  const { subtitle, statusLabel, statusTone } = details[state];

  return (
    <li key="notion" className="connector-row">
      <span className="connector-logo" aria-hidden>
        <ConnectorProviderIcon provider="notion" />
      </span>
      <div className="connector-main">
        <span className="connector-title-line">
          <span className="connector-name">Notion</span>
          <HoverTip
            tip={
              <>
                {t("settingsPanels.connectors.notionScopeDisclosure")}{" "}
                {t("settingsPanels.connectors.notionSearchDisclosure")}
              </>
            }
            width={360}
          >
            <button
              type="button"
              className="settings-row-info-affordance"
              aria-label={t("settingsPanels.connectors.notionScopeAria")}
              aria-describedby="notion-scope-disclosure"
            >
              <IconCircleInfo size={13} ariaHidden />
            </button>
          </HoverTip>
          <span id="notion-scope-disclosure" className="visually-hidden">
            {t("settingsPanels.connectors.notionScopeDisclosure")}{" "}
            {t("settingsPanels.connectors.notionSearchDisclosure")}
          </span>
        </span>
        <p className="connector-subtitle" title={subtitle}>
          {subtitle}
        </p>
      </div>
      <div className="connector-actions">
        {statusLabel ? (
          <span className="status-pill" data-tone={statusTone}>
            {statusLabel}
          </span>
        ) : null}
        <NotionConnectorActions
          state={state}
          connecting={connecting}
          disconnecting={disconnecting}
          onDisconnect={onDisconnect}
          onOpenConnectDialog={onOpenConnectDialog}
          onOpenReconnectDialog={onOpenReconnectDialog}
        />
      </div>
    </li>
  );
}

const CONNECT_TITLES = {
  google: {
    connect: "settingsPanels.connectors.titleConnectGoogle",
    add: "settingsPanels.connectors.titleAddGoogle",
  },
  linear: {
    connect: "settingsPanels.connectors.titleConnectLinear",
    add: "settingsPanels.connectors.titleAddLinear",
  },
  github: {
    connect: "settingsPanels.connectors.titleConnectGithub",
    add: "settingsPanels.connectors.titleAddGithub",
    reconnect: "settingsPanels.connectors.titleReconnectGithub",
  },
} as const satisfies Record<
  OAuthConnectorProvider,
  { connect: MessageKey; add: MessageKey; reconnect?: MessageKey }
>;

const CONNECT_TOASTS = {
  google: {
    connect: "settingsPanels.connectors.toastConnectGoogle",
    add: "settingsPanels.connectors.toastAddGoogle",
    reconnect: "settingsPanels.connectors.toastReconnectGoogle",
  },
  linear: {
    connect: "settingsPanels.connectors.toastConnectLinear",
    add: "settingsPanels.connectors.toastAddLinear",
    reconnect: "settingsPanels.connectors.toastReconnectLinear",
  },
  github: {
    connect: "settingsPanels.connectors.toastConnectGithub",
    add: "settingsPanels.connectors.toastAddGithub",
    reconnect: "settingsPanels.connectors.toastReconnectGithub",
  },
} as const satisfies Record<
  OAuthConnectorProvider,
  { connect: MessageKey; add: MessageKey; reconnect: MessageKey }
>;

/** True once an account holds every feature bundle its provider offers -
 * nothing left to add. */
function allBundlesGranted(
  policy: NonNullable<ReturnType<typeof useConnectorPolicy>["policy"]>,
  account: ConnectorAccount,
): boolean {
  if (account.provider === "notion") return true;
  return (
    bundlesFromScopes(policy, account.scopes, account.provider).length >=
    bundlesForProvider(policy, account.provider).length
  );
}

/** The name to show for an account outside its row (dialog titles, toasts):
 * the workspace name for Linear, falling back to the signed-in user's email
 * and then a generic label; the email for Google. */
function accountDisplayName(account: ConnectorAccount): string {
  if (account.provider === "linear") {
    return (
      account.workspaceName ||
      account.email ||
      translate("settingsPanels.connectors.linearWorkspaceFallback")
    );
  }
  if (account.provider === "notion") return "Notion";
  return account.email;
}

/** The login hint the connect flow escalates on: a Google email; the Linear
 * workspace's account id (Linear escalates by workspace, not by user email);
 * or the GitHub numeric user id (GitHub accounts are keyed by id, not login). */
function loginHintFor(account: ConnectorAccount): string {
  if (account.provider === "linear" || account.provider === "github") return account.accountId;
  return account.email;
}

function featureSummary(
  policy: NonNullable<ReturnType<typeof useConnectorPolicy>["policy"]>,
  account: ConnectorAccount,
): string {
  const features = grantedFeatureLabels(policy, account.scopes, account.provider);
  return features.length > 0
    ? translate("settingsPanels.connectors.featureSummary", {
        features: features.join(", ").toLowerCase(),
      })
    : "";
}

function linearNeedsScopeReconnect(
  policy: NonNullable<ReturnType<typeof useConnectorPolicy>["policy"]>,
  account: ConnectorAccount,
): boolean {
  return (
    account.provider === "linear" &&
    !scopesCoverBundles(policy, account.scopes, LINEAR_FULL_ACCESS_BUNDLES)
  );
}

function obsidianSubtitle(status: ObsidianStatus | null): string {
  if (!status?.connected) return translate("settingsPanels.connectors.obsidianBlurb");
  const vault = status.vaultName ?? translate("settingsPanels.connectors.vault");
  if (status.available === false)
    return translate("settingsPanels.connectors.vaultUnavailableAt", {
      vault,
      path: status.vaultPath,
    });
  return `${vault} · ${status.vaultPath}`;
}

function obsidianStatusMeta(status: ObsidianStatus | null) {
  if (!status?.connected) return null;
  if (status.available === false)
    return {
      label: translate("settingsPanels.connectors.vaultUnavailable"),
      tone: "warning",
    } as const;
  return { label: translate("settingsPanels.connectors.connected"), tone: "ok" } as const;
}

/** The connected row's one-liner: who is connected, then what Clovy may do. */
function accountSubtitle(
  policy: NonNullable<ReturnType<typeof useConnectorPolicy>["policy"]>,
  account: ConnectorAccount,
): string {
  if (account.provider === "linear") {
    if (account.status === "unavailable") {
      return translate("settingsPanels.connectors.linearUnavailable", {
        name: accountDisplayName(account),
      });
    }
    if (account.status === "reconnect_required" || linearNeedsScopeReconnect(policy, account)) {
      return translate("settingsPanels.connectors.linearReconnect", {
        name: accountDisplayName(account),
      });
    }
    return translate("settingsPanels.connectors.linearConnected", {
      name: accountDisplayName(account),
    });
  }
  const parts = [accountDisplayName(account)];
  const summary = featureSummary(policy, account);
  if (summary) parts.push(summary);
  return parts.join(" · ");
}

/** The connect dialog's body copy: what picking bundles here means, phrased
 * for a first connect ("this account"/"this workspace") or for adding scope
 * to an already-connected one. The sign-in surface and the kind of content
 * named swap per provider so the promise reads correctly for either. */
function connectDescription(
  provider: OAuthConnectorProvider,
  target: ConnectorAccount | null,
): string {
  const lead = target
    ? translate("settingsPanels.connectors.leadAdd", { name: accountDisplayName(target) })
    : provider === "linear"
      ? translate("settingsPanels.connectors.leadWorkspace")
      : translate("settingsPanels.connectors.leadAccount");
  const bodyKey = {
    google: "settingsPanels.connectors.connectBodyGoogle",
    github: "settingsPanels.connectors.connectBodyGithub",
    linear: "settingsPanels.connectors.connectBodyLinear",
  } as const satisfies Record<OAuthConnectorProvider, MessageKey>;
  return translate(bodyKey[provider], { lead });
}

/**
 * The Connectors settings page: a provider directory (one row per provider,
 * always listed) with a feature-bundle picker per provider, reconnect for
 * lapsed grants, and disconnect with optional provider-side revoke. A
 * Linear connects workspace-wide through the provider's official hosted MCP.
 * Browser use owns a separate browser profile with its own stored grant. Local mode only: tokens live in
 * the Mac's Keychain and provider calls originate on this device.
 */
export function ConnectorsSection() {
  const t = useT();
  const { browserUseEnabled } = useExperimentalFlags();
  const { policy, error: policyError } = useConnectorPolicy();
  const oauthProviders =
    policy?.providers.flatMap((definition) =>
      definition.enabled && definition.connectFlow === "oauth" && definition.id !== "notion"
        ? [definition.id as OAuthConnectorProvider]
        : [],
    ) ?? [];
  const notionEnabled =
    policy?.providers.some(
      (definition) =>
        definition.id === "notion" && definition.enabled && definition.connectFlow === "hosted_mcp",
    ) ?? false;
  const [accounts, setAccounts] = useState<ConnectorAccount[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notConfigured, setNotConfigured] = useState<ConnectorProvider | null>(null);

  const [connectProvider, setConnectProvider] = useState<OAuthConnectorProvider>("google");
  const [connectOpen, setConnectOpen] = useState(false);
  const [bundles, setBundles] = useState<ConnectorScopeBundle[]>([]);
  // The account we are adding scope to (single-account-per-provider
  // incremental auth), or null for a first-time connect. Its login hint
  // preselects that account/workspace so the backend's single-account guard
  // passes.
  const [connectTarget, setConnectTarget] = useState<ConnectorAccount | null>(null);
  // When true, the connect dialog was opened by a GitHub reconnect click. This
  // changes the dialog title and suppresses the bundle picker (the user already
  // chose bundles when they first connected; we reuse them as-is).
  const [connectIsReconnect, setConnectIsReconnect] = useState(false);
  const [connecting, setConnecting] = useState(false);
  // Device-code payload for an in-flight GitHub connect (null = not in device
  // flow or no code received yet).
  const [githubDeviceCode, setGithubDeviceCode] = useState<GitHubDeviceCodePayload | null>(null);
  const [codeCopied, setCodeCopied] = useState(false);
  const codeCopiedTimerRef = useRef<number>();
  // An inline error inside the connect dialog (device-flow specific errors
  // that keep the dialog open so the user can retry).
  const [connectError, setConnectError] = useState<string | null>(null);
  const [reconnectingId, setReconnectingId] = useState<string | null>(null);
  const [disconnectTarget, setDisconnectTarget] = useState<ConnectorAccount | null>(null);
  // Revoke defaults ON: disconnecting without it leaves the grant alive at
  // the provider AND deletes Clovy's copy of its tokens, so Clovy can never
  // revoke it afterward - the user is left cleaning up orphaned
  // authorizations in the provider's own settings. Opting out stays
  // available for a deliberate reconnect-soon disconnect.
  const [revoke, setRevoke] = useState(true);
  const [disconnecting, setDisconnecting] = useState(false);
  const [ghConnecting, setGhConnecting] = useState(false);
  const [obsidian, setObsidian] = useState<ObsidianStatus | null>(null);
  const [obsidianError, setObsidianError] = useState<string | null>(null);
  const [obsidianBusy, setObsidianBusy] = useState(false);
  const [obsidianDisconnecting, setObsidianDisconnecting] = useState(false);
  const [notionConnecting, setNotionConnecting] = useState(false);
  const [notionDisconnecting, setNotionDisconnecting] = useState(false);
  const [notionConnectOpen, setNotionConnectOpen] = useState(false);
  const [notionDialogMode, setNotionDialogMode] = useState<"connect" | "reconnect">("connect");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const notionOperationIdRef = useRef(0);
  const obsidianMeta = obsidianStatusMeta(obsidian);

  const refresh = useCallback(async () => {
    try {
      const list = await connectorsList();
      setAccounts(list);
      setLoadError(null);
    } catch (err) {
      setAccounts((current) => current ?? []);
      setLoadError(messageFromError(err));
    }
    try {
      setObsidian(await obsidianStatus());
      setObsidianError(null);
    } catch (err) {
      setObsidian((current) => current ?? { connected: false });
      setObsidianError(messageFromError(err));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void refresh();
    void listen(CONNECTORS_CHANGED_EVENT, () => void refresh()).then((cleanup) => {
      // Unmount can race the listen() promise — unsubscribe immediately
      // instead of leaking the listener.
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);

  // A signed-in GitHub CLI connects "GitHub (via gh)" on its own when there
  // is no GitHub account yet; the backend answers quickly and never throws.
  useEffect(() => {
    let cancelled = false;
    void connectorsGithubCliDetect()
      .then((account) => {
        if (account && !cancelled) void refresh();
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  // Listen for GitHub device-authorization codes while a GitHub connect is in
  // flight. The backend may re-emit the event (code renewal); we always render
  // the latest. We subscribe once for the lifetime of the component rather than
  // gating on `connecting`, so a code that arrives just before state flips is
  // never dropped.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen(GITHUB_DEVICE_CODE_EVENT, (event: { payload: GitHubDeviceCodePayload }) => {
      if (!cancelled) setGithubDeviceCode(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  async function runConnect(input: {
    provider: OAuthConnectorProvider;
    scopes: ConnectorScopeBundle[];
    loginHint?: string;
  }) {
    const account = await connectorsConnect({
      provider: input.provider,
      scopes: input.scopes,
      loginHint: input.loginHint,
    });
    // The Clovy-owned runtime reads connector state for every run, so a new
    // account needs no runtime apply step: refreshing Settings is enough.
    await refresh();
    return account;
  }

  async function connectLinear(target: ConnectorAccount | null = null) {
    if (connecting) return;
    setConnectProvider("linear");
    setConnectTarget(target);
    setConnectIsReconnect(target !== null);
    setConnectError(null);
    setNotConfigured(null);
    setConnectOpen(true);
    setConnecting(true);
    try {
      await runConnect({
        provider: "linear",
        scopes: ["linear_read", "linear_write"],
        loginHint: target ? loginHintFor(target) : undefined,
      });
      setConnectOpen(false);
      setConnectIsReconnect(false);
      toast.success(t(target ? CONNECT_TOASTS.linear.reconnect : CONNECT_TOASTS.linear.connect));
    } catch (err) {
      setConnectOpen(false);
      setConnectIsReconnect(false);
      if (isConnectorNotConfiguredError(err)) setNotConfigured("linear");
      else if (errorCode(err) !== "connector_connect_canceled") toast.error(messageFromError(err));
    } finally {
      setConnecting(false);
    }
  }

  // Open the connect dialog for a brand-new account of the given provider
  // (only offered when none is connected), or to add scope to the one
  // existing account.
  function openConnectNew(provider: OAuthConnectorProvider) {
    if (!policy) return;
    if (provider === "linear") {
      void connectLinear(null);
      return;
    }
    setConnectProvider(provider);
    setBundles(defaultBundlesForProvider(policy, provider));
    setConnectTarget(null);
    setConnectOpen(true);
  }

  function openAddAccess(account: ConnectorAccount) {
    if (account.provider === "notion" || !policy) return;
    // Preselect what the account already holds so the dialog reads as "add to
    // these"; the checkboxes the user adds are the new scopes.
    setConnectProvider(account.provider);
    setBundles(bundlesFromScopes(policy, account.scopes, account.provider));
    setConnectTarget(account);
    setConnectOpen(true);
  }

  async function submitConnect() {
    if (bundles.length === 0 || connecting) return;
    setNotConfigured(null);
    setConnectError(null);
    setGithubDeviceCode(null);
    setConnecting(true);
    try {
      await runConnect({
        provider: connectProvider,
        scopes: bundles,
        loginHint: connectTarget ? loginHintFor(connectTarget) : undefined,
      });
      setConnectOpen(false);
      setConnectIsReconnect(false);
      setGithubDeviceCode(null);
      const toasts = CONNECT_TOASTS[connectProvider];
      toast.success(
        t(connectIsReconnect ? toasts.reconnect : connectTarget ? toasts.add : toasts.connect),
      );
    } catch (err) {
      const code = errorCode(err);
      if (isConnectorNotConfiguredError(err)) {
        setNotConfigured(connectProvider);
        setConnectOpen(false);
      } else if (code === "connector_connect_canceled") {
        // A user-initiated cancel rejects the in-flight connect with this
        // code; that is the expected outcome of clicking Cancel, not an
        // error to surface.
        setGithubDeviceCode(null);
      } else if (code === "connector_github_device_expired") {
        // Code expired — let the user retry from the bundle picker.
        setGithubDeviceCode(null);
        setConnectError(t("settingsPanels.connectors.githubCodeExpired"));
      } else if (code === "connector_github_device_declined") {
        setGithubDeviceCode(null);
        setConnectError(t("settingsPanels.connectors.githubDeclined"));
      } else {
        setGithubDeviceCode(null);
        toast.error(messageFromError(err));
      }
    } finally {
      setConnecting(false);
    }
  }

  function copyDeviceCode() {
    if (!githubDeviceCode) return;
    void navigator.clipboard.writeText(githubDeviceCode.userCode).then(() => {
      setCodeCopied(true);
      if (codeCopiedTimerRef.current !== undefined) {
        window.clearTimeout(codeCopiedTimerRef.current);
      }
      codeCopiedTimerRef.current = window.setTimeout(() => {
        setCodeCopied(false);
        codeCopiedTimerRef.current = undefined;
      }, 2000);
    });
  }

  async function connectNotion() {
    if (notionConnecting || notionDisconnecting) return;
    const operationId = notionOperationIdRef.current + 1;
    notionOperationIdRef.current = operationId;
    setNotionConnecting(true);
    try {
      await notionConnectorConnect();
      // Notion tools are discovered fresh for each agent run, so the new
      // connection is available without applying a long-lived MCP config.
      if (operationId === notionOperationIdRef.current) {
        setNotionConnectOpen(false);
      }
      await refresh();
      if (operationId === notionOperationIdRef.current) {
        toast.success(t("settingsPanels.connectors.toastNotionConnected"));
      }
    } catch (err) {
      if (operationId === notionOperationIdRef.current) toast.error(messageFromError(err));
    } finally {
      if (operationId === notionOperationIdRef.current) setNotionConnecting(false);
    }
  }

  async function reconnectNotion() {
    await connectNotion();
  }

  async function disconnectNotion() {
    if (notionDisconnecting || notionConnecting) return;
    const operationId = notionOperationIdRef.current + 1;
    notionOperationIdRef.current = operationId;
    setNotionDisconnecting(true);
    try {
      await notionConnectorDisconnect();
      await refresh();
      if (operationId === notionOperationIdRef.current)
        toast.success(t("settingsPanels.connectors.toastNotionDisconnected"));
    } catch (err) {
      if (operationId === notionOperationIdRef.current) toast.error(messageFromError(err));
    } finally {
      if (operationId === notionOperationIdRef.current) setNotionDisconnecting(false);
    }
  }

  // Dismiss the connect dialog. While a connect is in flight this also aborts
  // the backend's wait, so Cancel and the close button work during that window
  // instead of being stuck until the flow resolves or times out.
  function dismissConnect() {
    if (connecting) void connectorsCancelConnect();
    setConnectOpen(false);
    setGithubDeviceCode(null);
    setConnectError(null);
    setConnectIsReconnect(false);
  }

  async function connectViaGh() {
    if (ghConnecting) return;
    setGhConnecting(true);
    try {
      const account = await connectorsGithubCliConnect();
      await refresh();
      if (account) toast.success(t("settingsPanels.connectors.ghConnected"));
    } catch (err) {
      toast.error(messageFromError(err));
    } finally {
      setGhConnecting(false);
    }
  }

  async function reconnect(account: ConnectorAccount) {
    if (account.provider === "notion" || !policy) return;
    setNotConfigured(null);
    if (account.viaGh) {
      await connectViaGh();
      return;
    }

    // GitHub uses the device-authorization flow: the device-code panel only
    // renders inside the connect dialog. Route GitHub reconnects through the
    // same dialog surface so the user sees the code. The dialog starts the
    // connect immediately (no extra click needed) using the account's existing
    // bundles and login hint — the reconnect flag suppresses the bundle picker
    // and adjusts the title and toast.
    if (account.provider === "github") {
      const accountBundles = bundlesFromScopes(policy, account.scopes, account.provider);
      setConnectProvider("github");
      setConnectTarget(account);
      setBundles(accountBundles);
      setConnectIsReconnect(true);
      setConnectError(null);
      setGithubDeviceCode(null);
      setConnectOpen(true);
      // Start the connect immediately — the user already clicked Reconnect.
      setConnecting(true);
      try {
        await runConnect({
          provider: "github",
          scopes: accountBundles,
          loginHint: loginHintFor(account),
        });
        setConnectOpen(false);
        setConnectIsReconnect(false);
        setGithubDeviceCode(null);
        toast.success(t(CONNECT_TOASTS.github.reconnect));
      } catch (err) {
        const code = errorCode(err);
        if (isConnectorNotConfiguredError(err)) {
          setNotConfigured("github");
          setConnectOpen(false);
          setConnectIsReconnect(false);
        } else if (code === "connector_connect_canceled") {
          setGithubDeviceCode(null);
        } else if (code === "connector_github_device_expired") {
          setGithubDeviceCode(null);
          setConnectError(t("settingsPanels.connectors.githubCodeExpired"));
        } else if (code === "connector_github_device_declined") {
          setGithubDeviceCode(null);
          setConnectError(t("settingsPanels.connectors.githubDeclined"));
        } else {
          setGithubDeviceCode(null);
          toast.error(messageFromError(err));
        }
      } finally {
        setConnecting(false);
      }
      return;
    }

    if (account.provider === "linear") {
      await connectLinear(account);
      return;
    }

    setReconnectingId(account.accountId);
    try {
      await runConnect({
        provider: account.provider,
        scopes: bundlesFromScopes(policy, account.scopes, account.provider),
        loginHint: loginHintFor(account),
      });
      toast.success(t(CONNECT_TOASTS[account.provider].reconnect));
    } catch (err) {
      if (isConnectorNotConfiguredError(err)) setNotConfigured(account.provider);
      else toast.error(messageFromError(err));
    } finally {
      setReconnectingId(null);
    }
  }

  async function confirmDisconnect() {
    const account = disconnectTarget;
    if (!account || disconnecting) return;
    setDisconnecting(true);
    try {
      const outcome = await connectorsDisconnect({
        accountId: account.accountId,
        // Clovy holds no gh credential to revoke.
        revoke: account.viaGh ? false : revoke,
      });
      await refresh();
      setDisconnectTarget(null);
      if (outcome?.providerRevocationConfirmed === false) {
        toast.warning(
          t("settingsPanels.connectors.disconnectedLocally", {
            name: accountDisplayName(account),
            provider: PROVIDER_NAMES[account.provider],
          }),
        );
      } else {
        toast.success(
          t("settingsPanels.connectors.disconnectedName", { name: accountDisplayName(account) }),
        );
      }
    } catch (err) {
      toast.error(messageFromError(err));
    } finally {
      setDisconnecting(false);
    }
  }

  async function chooseObsidianVault() {
    if (obsidianBusy) return;
    setObsidianError(null);
    setObsidianBusy(true);
    try {
      const selected = await openFileDialog({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      setObsidian(await obsidianConfigure(selected));
      await refresh();
      toast.success(t("settingsPanels.connectors.toastObsidianConnected"));
    } catch (err) {
      setObsidianError(messageFromError(err));
    } finally {
      setObsidianBusy(false);
    }
  }

  async function disconnectObsidian() {
    if (obsidianBusy) return;
    setObsidianError(null);
    setObsidianBusy(true);
    setObsidianDisconnecting(true);
    try {
      await obsidianDisconnect();
      await refresh();
      toast.success(t("settingsPanels.connectors.toastObsidianDisconnected"));
    } catch (err) {
      setObsidianError(messageFromError(err));
    } finally {
      setObsidianBusy(false);
      setObsidianDisconnecting(false);
    }
  }

  function toggleBundle(bundle: ConnectorScopeBundle, checked: boolean) {
    if (!policy) return;
    setBundles((current) => {
      const next = new Set(current);
      if (checked) next.add(bundle);
      else next.delete(bundle);
      return bundlesForProvider(policy, connectProvider).filter((entry) => next.has(entry));
    });
  }

  return (
    <section className="settings-group" aria-labelledby="plugins-heading">
      <SettingsPageHeader
        id="plugins-heading"
        title={t("settingsPanels.connectors.title")}
        blurb={t("settingsPanels.connectors.blurb")}
      />

      {notConfigured ? (
        <InlineNotice
          tone="info"
          body={t("settingsPanels.connectors.notConfigured", {
            provider: PROVIDER_NAMES[notConfigured],
          })}
          aria-label={t("settingsPanels.connectors.notConfiguredAria")}
        />
      ) : null}
      {loadError ? (
        <InlineNotice
          tone="warning"
          body={loadError}
          aria-label={t("settingsPanels.connectors.loadErrorAria")}
        />
      ) : null}
      {policyError ? (
        <InlineNotice
          tone="warning"
          body={messageFromError(policyError)}
          aria-label={t("settingsPanels.connectors.policyErrorAria")}
        />
      ) : null}
      {obsidianError ? (
        <InlineNotice
          tone="warning"
          body={obsidianError}
          aria-label={t("settingsPanels.connectors.obsidianErrorAria")}
        />
      ) : null}

      <div className="settings-card connectors-card">
        <ul className="connectors-list">
          <li className="connector-row">
            <span className="connector-logo" aria-hidden>
              <ConnectorProviderIcon provider="obsidian" />
            </span>
            <div className="connector-main">
              <span className="connector-name">Obsidian</span>
              <p className="connector-subtitle" title={obsidian?.vaultPath}>
                {obsidianSubtitle(obsidian)}
              </p>
            </div>
            <div className="connector-actions">
              {obsidianMeta ? (
                <span className="status-pill" data-tone={obsidianMeta.tone}>
                  {obsidianMeta.label}
                </span>
              ) : null}
              <button
                type="button"
                className="btn btn-secondary"
                aria-label={
                  obsidian?.connected
                    ? t("settingsPanels.connectors.changeObsidianVault")
                    : t("settingsPanels.connectors.connectObsidian")
                }
                disabled={obsidian === null || obsidianBusy}
                aria-busy={obsidianBusy || undefined}
                onClick={() => void chooseObsidianVault()}
              >
                {obsidian?.connected
                  ? t("settingsPanels.connectors.changeVault")
                  : t("settingsPanels.connectors.connect")}
              </button>
              {obsidian?.connected ? (
                <button
                  type="button"
                  className="btn btn-ghost"
                  aria-label={t("settingsPanels.connectors.disconnectObsidian")}
                  disabled={obsidianBusy}
                  aria-busy={obsidianDisconnecting || undefined}
                  onClick={() => void disconnectObsidian()}
                >
                  {obsidianDisconnecting
                    ? t("settingsPanels.connectors.disconnecting")
                    : t("settingsPanels.connectors.disconnect")}
                </button>
              ) : null}
            </div>
          </li>
          {policy
            ? oauthProviders.map((provider) => {
                const account = accounts?.find((entry) => entry.provider === provider) ?? null;
                const name = PROVIDER_NAMES[provider];
                const effectiveStatus =
                  account?.status === "connected" && linearNeedsScopeReconnect(policy, account)
                    ? "reconnect_required"
                    : account?.status;
                const status = effectiveStatus
                  ? accountStatusMeta(effectiveStatus, provider)
                  : null;
                const subtitle = account
                  ? accountSubtitle(policy, account)
                  : t(PROVIDER_BLURB_KEYS[provider]);
                const reconnecting =
                  account !== null &&
                  (reconnectingId === account.accountId ||
                    (provider === connectProvider && connectIsReconnect && connecting));
                const connectingThisProvider =
                  account === null && connecting && connectProvider === provider;
                const showAddAccess =
                  account !== null &&
                  provider !== "linear" &&
                  (provider === "google" || !allBundlesGranted(policy, account));
                return (
                  <li key={provider} className="connector-row">
                    <span className="connector-logo" aria-hidden>
                      <ConnectorProviderIcon provider={provider} />
                    </span>
                    <div className="connector-main">
                      <span className="connector-title-line">
                        <span className="connector-name">{name}</span>
                        {account?.viaGh ? (
                          <span
                            className="connector-source-tag"
                            title={t("settingsPanels.connectors.viaGhTip")}
                          >
                            {t("settingsPanels.connectors.viaGh")}
                          </span>
                        ) : null}
                      </span>
                      <p className="connector-subtitle" title={subtitle}>
                        {subtitle}
                      </p>
                    </div>
                    <div className="connector-actions">
                      {account && status ? (
                        <span
                          className="status-pill"
                          data-tone={status.tone === "ok" ? "ok" : "warning"}
                          title={status.blurb}
                        >
                          {status.label}
                        </span>
                      ) : null}
                      {!account && provider === "github" ? (
                        <button
                          type="button"
                          className="btn btn-ghost"
                          aria-label={t("settingsPanels.connectors.connectWithGhName", { name })}
                          disabled={accounts === null || ghConnecting || connectingThisProvider}
                          aria-busy={ghConnecting || undefined}
                          onClick={() => void connectViaGh()}
                        >
                          {ghConnecting
                            ? t("settingsPanels.connectors.connecting")
                            : t("settingsPanels.connectors.connectWithGh")}
                        </button>
                      ) : null}
                      {!account ? (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          aria-label={t("settingsPanels.connectors.connectName", { name })}
                          disabled={accounts === null || connectingThisProvider}
                          aria-busy={connectingThisProvider || undefined}
                          onClick={() => openConnectNew(provider)}
                        >
                          {connectingThisProvider
                            ? t("settingsPanels.connectors.connecting")
                            : t("settingsPanels.connectors.connect")}
                        </button>
                      ) : (
                        <>
                          {effectiveStatus === "reconnect_required" ? (
                            <button
                              type="button"
                              className="btn btn-secondary"
                              aria-label={t("settingsPanels.connectors.reconnectName", { name })}
                              disabled={reconnectingId !== null || reconnecting}
                              aria-busy={reconnecting || undefined}
                              onClick={() => void reconnect(account)}
                            >
                              {reconnecting
                                ? t("settingsPanels.connectors.reconnecting")
                                : t("settingsPanels.connectors.reconnect")}
                            </button>
                          ) : showAddAccess ? (
                            <button
                              type="button"
                              className="btn btn-secondary"
                              onClick={() => openAddAccess(account)}
                            >
                              {t("settingsPanels.connectors.addAccess")}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className="btn btn-ghost"
                            aria-label={t("settingsPanels.connectors.disconnectName", { name })}
                            onClick={() => {
                              setRevoke(true);
                              setDisconnectTarget(account);
                            }}
                          >
                            {t("settingsPanels.connectors.disconnect")}
                          </button>
                        </>
                      )}
                    </div>
                  </li>
                );
              })
            : null}
          {notionEnabled ? (
            <NotionConnectorRow
              account={accounts?.find((entry) => entry.provider === "notion") ?? null}
              connecting={accounts === null || notionConnecting}
              disconnecting={notionDisconnecting}
              onDisconnect={() => void disconnectNotion()}
              onOpenConnectDialog={() => {
                setNotionDialogMode("connect");
                setNotionConnectOpen(true);
              }}
              onOpenReconnectDialog={() => {
                setNotionDialogMode("reconnect");
                setNotionConnectOpen(true);
              }}
            />
          ) : null}
          <li className="connectors-advanced-row">
            <button
              type="button"
              className="connectors-advanced-toggle"
              aria-expanded={advancedOpen}
              onClick={() => setAdvancedOpen((open) => !open)}
            >
              <span>{t("settingsPanels.connectors.advanced")}</span>
              <IconChevronDownSmall
                size={14}
                aria-hidden
                className="connectors-advanced-chevron"
                data-open={advancedOpen || undefined}
              />
            </button>
          </li>
          {advancedOpen && browserUseEnabled ? <BrowserUseControl /> : null}
          {advancedOpen ? <ComputerUseControl /> : null}
        </ul>
      </div>

      <Dialog
        open={connectOpen}
        onClose={dismissConnect}
        title={
          connectIsReconnect && connectProvider === "linear"
            ? t("settingsPanels.connectors.titleReconnectLinear")
            : connectIsReconnect && connectProvider === "github" && CONNECT_TITLES.github.reconnect
              ? t(CONNECT_TITLES.github.reconnect)
              : connectTarget
                ? t(CONNECT_TITLES[connectProvider].add)
                : t(CONNECT_TITLES[connectProvider].connect)
        }
        description={
          connecting && connectProvider === "linear"
            ? connectIsReconnect
              ? t("settingsPanels.connectors.linearFinishReconnect")
              : t("settingsPanels.connectors.linearFinishConnect")
            : githubDeviceCode && connectProvider === "github"
              ? undefined
              : connectDescription(connectProvider, connectTarget)
        }
        footer={
          (connecting && connectProvider === "linear") ||
          (githubDeviceCode && connectProvider === "github") ? (
            // Device-code waiting state: only a cancel affordance. The connect
            // resolves (or errors) on its own once the user approves on GitHub.
            // Cancel calls connectorsCancelConnect via dismissConnect.
            <button type="button" className="primary-action" onClick={dismissConnect}>
              {t("common.cancel")}
            </button>
          ) : (
            <>
              <button type="button" className="primary-action" onClick={dismissConnect}>
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className="primary-action primary-solid"
                disabled={bundles.length === 0 || connecting}
                aria-busy={connecting || undefined}
                onClick={() => void submitConnect()}
              >
                {connecting && connectProvider !== "github"
                  ? t("settingsPanels.connectors.waitingForBrowser")
                  : t("settingsPanels.connectors.connect")}
              </button>
            </>
          )
        }
      >
        {connecting && connectProvider === "linear" ? null : githubDeviceCode &&
          connectProvider === "github" ? (
          // GitHub device-authorization flow: show the user code the user must
          // enter at github.com/login/device. The backend has already opened
          // the page; the code display and link are the fallback.
          <div className="github-device-code-panel">
            <p className="github-device-code-label">
              {t("settingsPanels.connectors.githubEnterCode")}
            </p>
            <div className="github-device-code-row">
              <span className="github-device-code-value">{githubDeviceCode.userCode}</span>
              <button
                type="button"
                className="btn btn-secondary github-device-code-copy"
                aria-label={
                  codeCopied
                    ? t("settingsPanels.connectors.codeCopied")
                    : t("settingsPanels.connectors.copyCode")
                }
                onClick={copyDeviceCode}
              >
                <CopyStateIcon copied={codeCopied} />
                {t("settingsPanels.connectors.copyCode")}
              </button>
            </div>
            <p className="github-device-code-status">
              {t("settingsPanels.connectors.githubWaiting")}
            </p>
          </div>
        ) : (
          <>
            {connectError ? (
              <InlineNotice
                tone="warning"
                body={connectError}
                aria-label={t("settingsPanels.connectors.connectErrorAria")}
              />
            ) : null}
            <div className="connectors-bundle-list">
              {(policy ? bundlesForProvider(policy, connectProvider) : []).map((bundle) => {
                const meta = bundleMeta(bundle);
                return (
                  <label
                    key={bundle}
                    className="connectors-bundle-option"
                    htmlFor={`connectors-bundle-${bundle}`}
                  >
                    <Checkbox
                      id={`connectors-bundle-${bundle}`}
                      checked={bundles.includes(bundle)}
                      disabled={connecting}
                      onChange={(event) => toggleBundle(bundle, event.currentTarget.checked)}
                    />
                    <span className="connectors-bundle-copy">
                      <span className="connectors-bundle-label">{meta.label}</span>
                      <span className="connectors-bundle-description">{meta.description}</span>
                    </span>
                  </label>
                );
              })}
            </div>
          </>
        )}
      </Dialog>

      <Dialog
        open={disconnectTarget !== null}
        onClose={() => {
          if (!disconnecting) setDisconnectTarget(null);
        }}
        title={t("settingsPanels.connectors.disconnectTitle", {
          name: disconnectTarget ? accountDisplayName(disconnectTarget) : "",
        })}
        description={
          disconnectTarget?.viaGh
            ? t("settingsPanels.connectors.disconnectGhDescription")
            : t("settingsPanels.connectors.disconnectDescription")
        }
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={() => setDisconnectTarget(null)}
              disabled={disconnecting}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid primary-destructive"
              disabled={disconnecting}
              aria-busy={disconnecting || undefined}
              onClick={() => void confirmDisconnect()}
            >
              {disconnecting
                ? t("settingsPanels.connectors.disconnecting")
                : t("settingsPanels.connectors.disconnect")}
            </button>
          </>
        }
      >
        {disconnectTarget?.viaGh ? null : (
          <label className="connectors-revoke-option" htmlFor="connectors-revoke">
            <Checkbox
              id="connectors-revoke"
              checked={revoke}
              disabled={disconnecting}
              onChange={(event) => setRevoke(event.currentTarget.checked)}
            />
            {disconnectTarget
              ? t("settingsPanels.connectors.revokeWith", {
                  provider: PROVIDER_NAMES[disconnectTarget.provider],
                })
              : t("settingsPanels.connectors.revokeWithProvider")}
          </label>
        )}
      </Dialog>

      <Dialog
        open={notionConnectOpen}
        onClose={() => {
          if (!notionConnecting) setNotionConnectOpen(false);
        }}
        title={
          notionDialogMode === "reconnect"
            ? t("settingsPanels.connectors.reconnectNotion")
            : t("settingsPanels.connectors.connectNotion")
        }
        description={t("settingsPanels.connectors.notionDialogDescription")}
        disableBackdropClose={notionConnecting}
        closeDisabled={notionConnecting}
        footer={
          <Fragment>
            <button
              type="button"
              className="primary-action"
              onClick={() => setNotionConnectOpen(false)}
              disabled={notionConnecting}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              disabled={notionConnecting}
              aria-busy={notionConnecting || undefined}
              onClick={() => {
                if (notionDialogMode === "reconnect") {
                  void reconnectNotion();
                } else {
                  void connectNotion();
                }
              }}
            >
              {notionConnecting
                ? t("settingsPanels.connectors.waitingForBrowser")
                : t("common.continue")}
            </button>
          </Fragment>
        }
      >
        <div className="connectors-notion-disclosures">
          <p className="connectors-notion-disclosure">
            {t("settingsPanels.connectors.notionScopeDisclosure")}
          </p>
          <p className="connectors-notion-disclosure">
            {t("settingsPanels.connectors.notionSearchDisclosure")}
          </p>
        </div>
      </Dialog>
    </section>
  );
}
