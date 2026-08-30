import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconShuffle } from "central-icons/IconShuffle";
import { useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { toast } from "../ui/Toaster";
import { hasLiveSubscription } from "../../lib/account-gate";
import { errorCode } from "../../lib/errors";
import {
  MAX_GRANT_HOSTED_POLL_TIMEOUT_MS,
  MAX_UPGRADE_BROWSER_STATUS,
  MAX_UPGRADE_BUSY_LABEL,
  MAX_UPGRADE_CHARGE_CONFIRM_BODY,
  MAX_UPGRADE_CONFIRM_BODY,
  MAX_UPGRADE_CONFIRM_LABEL,
  MAX_UPGRADE_CONFIRM_TITLE,
  MAX_UPGRADE_READY_STATUS,
  MAX_UPGRADE_WAITING_STATUS,
  type MaxGrantWait,
  accountLooksPreGrant,
  beginMaxGrantWait,
  clearMaxGrantWait,
  isHostedMaxUpgradeFallbackError,
  isMaxGrantWaitCurrent,
  markMaxGrantWaitSlow,
  isMaxUpgradeWaitStatus,
  markMaxGrantWaitWaiting,
  maxGrantLanded,
  maxGrantWaitForAccount,
  maxUpgradeSlowStatus,
  maxUpgradeWaitStatus,
  pollForMaxGrant,
} from "../../lib/max-upgrade";
import {
  BILLING_DEMO_FIXTURES,
  BILLING_DEMO_ORDER,
  useForcedBillingPlan,
} from "../../lib/billing-demo";
import {
  osAccountsCancelLogin,
  osAccountsChangePlan,
  osAccountsLogin,
  osAccountsLogout,
  osAccountsOpenPortal,
  osAccountsUpgrade,
  osAccountsUpgradeSession,
} from "../../lib/tauri";
import type { AccountStatus, SubscriptionPlan } from "../../lib/tauri";
import { AccountAvatar, useAccountAvatar } from "./AccountAvatar";

const FREE_PLAN_NAME = "Free plan";
const PRO_PLAN_NAME = "Pro plan";
const MAX_PLAN_NAME = "Max plan";
export const FREE_PLAN_CREDITS = 4000;

type Props = {
  account: AccountStatus;
  loading: boolean;
  onAccountChanged: (next: AccountStatus) => void;
  onRefresh: () => Promise<AccountStatus | undefined>;
};

export function AccountSettings({ account, loading, onAccountChanged, onRefresh }: Props) {
  return (
    <div className="settings-page">
      <header className="settings-header">
        <h1 className="settings-title">Account</h1>
        <p className="settings-description">
          {account.localDev
            ? "Local mode is active. Clovy uses your local Clovy API without OpenSoftware sign-in or billing."
            : "Sign in with OpenSoftware to use your shared identity and balance across the network."}
        </p>
      </header>

      <AccountSettingsSection
        account={account}
        loading={loading}
        onAccountChanged={onAccountChanged}
        onRefresh={onRefresh}
      />
      {account.localDev ? null : <BillingSettingsSection account={account} onRefresh={onRefresh} />}
    </div>
  );
}

export function AccountSettingsSection({ account, loading, onAccountChanged }: Props) {
  const [busy, setBusy] = useState(false);
  const [avatarBusy, setAvatarBusy] = useState(false);
  const [accountStatus, setAccountStatus] = useState<string>();
  const accountRef = useRef(account);
  const mountedRef = useRef(true);
  accountRef.current = account;
  const { localOnly: avatarLocalOnly, refresh: refreshAvatar } = useAccountAvatar(account);
  const canManageAvatar = account.localDev || Boolean(account.signedIn && account.user?.id?.trim());

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  async function handleRefreshAvatar() {
    setAvatarBusy(true);
    try {
      const user = await refreshAvatar();
      if (user) {
        // Propagate to App-level account state before the mounted guard: the
        // callback is safe post-unmount and skipping it would strand the
        // local-only copy until the next focus refresh.
        const currentAccount = accountRef.current;
        if (currentAccount.signedIn && currentAccount.user?.id === user.id) {
          onAccountChanged({ ...currentAccount, user });
        }
      }
      if (!mountedRef.current) return;
      toast.success(
        accountRef.current.localDev ? "Avatar updated on this device" : "Avatar updated everywhere",
      );
    } catch (error) {
      if (mountedRef.current && (accountRef.current.signedIn || accountRef.current.localDev)) {
        const code = errorCode(error);
        if (code === "account_permission_required") {
          toast.warning(
            "Avatar changed on this device, but it couldn't sync. Sign out and sign in again to update your account permissions.",
          );
        } else if (code === "avatar_sync_unavailable") {
          toast.warning(
            "Avatar changed on this device, but syncing isn't available in this OS Accounts environment yet.",
          );
        } else {
          toast.error(messageFromError(error));
        }
      }
    } finally {
      if (mountedRef.current) setAvatarBusy(false);
    }
  }

  async function handleSignIn() {
    setBusy(true);
    setAccountStatus("Opening your browser to sign in…");
    try {
      const next = await osAccountsLogin();
      onAccountChanged(next);
      setAccountStatus(next.signedIn ? `Signed in as ${displayName(next)}.` : undefined);
    } catch (error) {
      setAccountStatus(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }

  async function handleCancel() {
    // Aborts the waiting os_accounts_login; that promise then rejects with
    // "login_canceled", and handleSignIn's catch/finally resets status + busy.
    try {
      await osAccountsCancelLogin();
    } catch (error) {
      setAccountStatus(messageFromError(error));
    }
  }

  async function handleSignOut() {
    setBusy(true);
    try {
      await osAccountsLogout({ clearBrowserSession: true });
      onAccountChanged({ signedIn: false, configured: account.configured });
      setAccountStatus("Signed out.");
    } catch (error) {
      setAccountStatus(messageFromError(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="settings-group" aria-labelledby="account-heading">
      <h2 id="account-heading" className="settings-group-heading">
        Account
      </h2>
      {accountStatus ? <p className="settings-status">{accountStatus}</p> : null}
      <div className="settings-card">
        <div className="settings-rows">
          <div className="settings-row">
            <div className="settings-row-info">
              <h3 className="settings-row-title">
                {account.localDev
                  ? "Local mode"
                  : loading
                    ? "Checking sign-in..."
                    : account.signedIn
                      ? displayName(account)
                      : "Not signed in"}
              </h3>
              <p className="settings-row-description">
                {account.localDev
                  ? "Requests use your local Clovy API. No OpenSoftware account is used."
                  : account.signedIn
                    ? (account.user?.email ?? `@${account.user?.handle ?? "account"}`)
                    : account.configured
                      ? "Your login is managed by OpenSoftware."
                      : "OpenSoftware sign-in is not configured for this build."}
              </p>
            </div>
            <div className="settings-row-control">
              {account.localDev ? null : account.signedIn ? (
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={busy || avatarBusy}
                  onClick={() => void handleSignOut()}
                >
                  Sign out
                </button>
              ) : busy ? (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => void handleCancel()}
                >
                  Cancel
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={loading || !account.configured}
                  onClick={() => void handleSignIn()}
                >
                  Sign in with OpenSoftware
                </button>
              )}
            </div>
          </div>
          {canManageAvatar ? (
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">Avatar</h3>
                <p className="settings-row-description">
                  {account.localDev
                    ? "A generated pattern saved on this device."
                    : avatarLocalOnly
                      ? "This pattern is saved only on this device."
                      : "A generated pattern synced with your OpenSoftware account."}
                </p>
              </div>
              <div className="settings-row-control">
                <AccountAvatar account={account} className="account-avatar-preview" />
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={avatarBusy || busy}
                  onClick={() => void handleRefreshAvatar()}
                >
                  <IconShuffle size={14} />
                  Refresh
                </button>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}

export function BillingSettingsSection({
  account,
  onRefresh,
}: Pick<Props, "account" | "onRefresh">) {
  const [refreshing, setRefreshing] = useState(false);
  const [maxGrantWait, setMaxGrantWait] = useState<MaxGrantWait | undefined>(() =>
    maxGrantWaitForAccount(account.user?.id),
  );
  const [billingStatus, setBillingStatus] = useState<string | undefined>(() => {
    if (!maxGrantWait) return undefined;
    if (maxGrantWait.phase === "browser") return MAX_UPGRADE_BROWSER_STATUS;
    return maxGrantWait.phase === "slow"
      ? maxUpgradeSlowStatus(maxGrantWait)
      : MAX_UPGRADE_WAITING_STATUS;
  });
  const [spins, setSpins] = useState(0);
  // The plan awaiting an explicit confirm. A plan change can charge the saved
  // card, so it never starts straight from the card CTA.
  const [planToConfirm, setPlanToConfirm] = useState<SubscriptionPlan | null>(null);
  const [confirmError, setConfirmError] = useState<string>();
  // Whether the confirm dialog has switched to the PATCH transport's
  // charge-now copy after a hosted capability signal. The next confirm under
  // that copy is what authorizes the saved-card charge.
  const [chargeNowUpgrade, setChargeNowUpgrade] = useState(false);
  const demoPlan = useForcedBillingPlan();

  async function handleUpgrade(plan: SubscriptionPlan) {
    try {
      await osAccountsUpgrade(plan);
      setBillingStatus("Opened checkout in your browser.");
    } catch (error) {
      setBillingStatus(messageFromError(error));
    }
  }

  // Hosted upgrade for a paid subscriber (currently Pro -> Max), run from the
  // confirm dialog only. When this OS Accounts deploy cannot host the browser
  // flow, the dialog switches to the charge-now copy and the PATCH waits for
  // one more explicit confirm - hosted-copy consent never authorizes a
  // saved-card charge. Only the credit grant poll announces Max.
  async function handleChangePlan(plan: SubscriptionPlan) {
    // A wait can begin on a coexisting surface while this confirm sits open
    // (the funding notice, the sidebar chip). Never stack a second purchase
    // on it; adopt the wait and show its status. A slow wait stays
    // retryable - the dispatch below supersedes it.
    const pendingWait = maxGrantWaitForAccount(account.user?.id);
    if (pendingWait && pendingWait.phase !== "slow") {
      setMaxGrantWait(pendingWait);
      setBillingStatus(
        pendingWait.phase === "browser" ? MAX_UPGRADE_BROWSER_STATUS : MAX_UPGRADE_WAITING_STATUS,
      );
      return;
    }
    const baselineCredits = account.balance?.credits ?? 0;
    const chargeNow = chargeNowUpgrade;
    let alreadyOnPlan = false;
    try {
      if (chargeNow) {
        await osAccountsChangePlan(plan);
      } else {
        await osAccountsUpgradeSession(plan);
      }
    } catch (error) {
      const code = errorCode(error);
      if (code === "already_on_plan") {
        alreadyOnPlan = true;
      } else if (code === "subscription_required") {
        setBillingStatus(messageFromError(error));
        await onRefresh();
        return;
      } else if (!chargeNow && isHostedMaxUpgradeFallbackError(error)) {
        // Definitive capability signal: nothing was charged. Swap the dialog
        // to the charge-now copy and keep it open for a fresh confirm.
        setConfirmError(undefined);
        setChargeNowUpgrade(true);
        throw error;
      } else {
        // Keep the dialog open (ConfirmDialog swallows the rethrow but stays
        // up) and show the failure inside it, next to the retry affordance.
        setConfirmError(messageFromError(error));
        throw error;
      }
    }
    if (alreadyOnPlan) {
      // The server already has the plan. One refresh decides between a grant
      // still landing (poll) and a long-settled Max account, where a poll
      // could never succeed and the card must re-derive from the snapshot.
      const refreshed = await onRefresh();
      if (!accountLooksPreGrant(refreshed, baselineCredits)) {
        // Settled: any wait for this account is obsolete and must not keep
        // suppressing the card's plan claim or upgrade CTAs. A retry
        // dispatched from the slow phase lands here.
        const staleWait = maxGrantWaitForAccount(account.user?.id);
        if (staleWait) clearMaxGrantWait(staleWait);
        setMaxGrantWait(undefined);
        setBillingStatus(undefined);
        return;
      }
    }
    const hostedReview = !chargeNow && !alreadyOnPlan;
    const grantWait = beginMaxGrantWait(
      baselineCredits,
      account.user?.id,
      hostedReview ? "browser" : "waiting",
    );
    setMaxGrantWait(grantWait);
    setBillingStatus(hostedReview ? MAX_UPGRADE_BROWSER_STATUS : MAX_UPGRADE_WAITING_STATUS);
    void pollForMaxGrant(
      onRefresh,
      baselineCredits,
      hostedReview ? { timeoutMs: MAX_GRANT_HOSTED_POLL_TIMEOUT_MS } : {},
    ).then((landed) => {
      // Ignore a stale poll from an earlier attempt or one superseded by a
      // manual refresh that already observed the grant.
      if (!isMaxGrantWaitCurrent(grantWait)) return;
      if (landed) {
        clearMaxGrantWait(grantWait);
        setMaxGrantWait(undefined);
        setBillingStatus(MAX_UPGRADE_READY_STATUS);
      } else {
        markMaxGrantWaitSlow(grantWait);
        setBillingStatus(maxUpgradeSlowStatus(grantWait));
      }
    });
  }

  useEffect(() => {
    if (maxGrantWait && maxGrantWait.accountId !== account.user?.id) {
      clearMaxGrantWait(maxGrantWait);
      setMaxGrantWait(undefined);
      setBillingStatus(undefined);
      return;
    }
    // Reconcile the cached wait against the shared record: an upgrade can
    // start, supersede, or be cancelled on a coexisting surface (the funding
    // notice, the sidebar chip), and this card must not keep offering - or
    // suppressing - the purchase path from a stale copy. Runs on every
    // account refresh tick.
    const currentWait = maxGrantWaitForAccount(account.user?.id);
    if (maxGrantWait !== currentWait) {
      setMaxGrantWait(currentWait);
      setBillingStatus(
        currentWait === undefined
          ? undefined
          : currentWait.phase === "browser"
            ? MAX_UPGRADE_BROWSER_STATUS
            : currentWait.phase === "slow"
              ? maxUpgradeSlowStatus(currentWait)
              : MAX_UPGRADE_WAITING_STATUS,
      );
      return;
    }
    if (!maxGrantWait) return;
    // A coexisting surface's poll advances the shared wait's phase by
    // in-place mutation, which the identity check above cannot see. Swap a
    // stale phase line for the live one - and only a phase line, never an
    // error or the ready announcement.
    const phaseCopy = maxUpgradeWaitStatus(maxGrantWait);
    setBillingStatus((status) =>
      status !== undefined && status !== phaseCopy && isMaxUpgradeWaitStatus(status)
        ? phaseCopy
        : status,
    );
    if (maxGrantWait.phase === "browser" && account.subscription?.plan === "max") {
      markMaxGrantWaitWaiting(maxGrantWait);
      setBillingStatus(MAX_UPGRADE_WAITING_STATUS);
    }
    if (!maxGrantLanded(account, maxGrantWait.baselineCredits)) return;
    clearMaxGrantWait(maxGrantWait);
    setMaxGrantWait(undefined);
    setBillingStatus(MAX_UPGRADE_READY_STATUS);
  }, [account, maxGrantWait]);

  async function handleManageSubscription() {
    try {
      await osAccountsOpenPortal();
      setBillingStatus("Opened your account portal in the browser.");
    } catch (error) {
      setBillingStatus(messageFromError(error));
    }
  }

  async function handleRefresh() {
    setRefreshing(true);
    setSpins((turns) => turns + 1);
    try {
      const next = await onRefresh();
      const grantWait = maxGrantWait;
      if (grantWait !== undefined) {
        if (maxGrantLanded(next, grantWait.baselineCredits)) {
          clearMaxGrantWait(grantWait);
          setMaxGrantWait(undefined);
          setBillingStatus(MAX_UPGRADE_READY_STATUS);
        } else if (grantWait.phase === "browser" && next?.subscription?.plan === "max") {
          markMaxGrantWaitWaiting(grantWait);
          setBillingStatus(MAX_UPGRADE_WAITING_STATUS);
        } else {
          // An adopted wait's phase can advance under another surface's poll
          // (browser -> waiting -> slow) without notifying this one; the copy
          // snapshotted at mount would otherwise stick. Re-derive it here so
          // an explicit refresh always reflects the live phase.
          setBillingStatus(
            grantWait.phase === "browser"
              ? MAX_UPGRADE_BROWSER_STATUS
              : grantWait.phase === "slow"
                ? maxUpgradeSlowStatus(grantWait)
                : MAX_UPGRADE_WAITING_STATUS,
          );
        }
        return;
      }
      setBillingStatus(undefined);
    } catch (error) {
      setBillingStatus(messageFromError(error));
    } finally {
      setRefreshing(false);
    }
  }

  // __billingDemo(...) parks the card in a fixture state (or "all" stacks every
  // variant) for design work. Every fixture reference is gated on
  // import.meta.env.DEV so the bundler drops BILLING_DEMO_FIXTURES (and the six
  // fixture accounts) from production — demoPlan is always null there anyway,
  // since the console command is never registered. See lib/billing-demo.ts.
  const demoAccount =
    import.meta.env.DEV && demoPlan && demoPlan !== "all"
      ? BILLING_DEMO_FIXTURES[demoPlan].account
      : undefined;
  const cardProps = {
    refreshing,
    spins,
    maxGrantPending: maxGrantWait !== undefined,
    maxGrantRetry: maxGrantWait?.phase === "slow",
    onRefresh: () => void handleRefresh(),
    onUpgrade: (plan: SubscriptionPlan) => void handleUpgrade(plan),
    // Confirm first because the change may charge the saved card.
    onChangePlan: (plan: SubscriptionPlan) => {
      setConfirmError(undefined);
      setChargeNowUpgrade(false);
      setPlanToConfirm(plan);
    },
    onManage: () => void handleManageSubscription(),
  };

  return (
    <section className="settings-group" aria-labelledby="billing-heading">
      <h2 id="billing-heading" className="settings-group-heading">
        Billing
      </h2>
      <p className="settings-group-description">
        Manage usage and subscription details in OpenSoftware.
      </p>
      {billingStatus ? <p className="settings-status">{billingStatus}</p> : null}
      {import.meta.env.DEV && demoPlan === "all" ? (
        <div className="billing-demo-gallery">
          {BILLING_DEMO_ORDER.map((key) => (
            <div className="billing-demo-variant" key={key}>
              <p className="billing-demo-label">{BILLING_DEMO_FIXTURES[key].label}</p>
              <BillingCard account={BILLING_DEMO_FIXTURES[key].account} {...cardProps} />
            </div>
          ))}
        </div>
      ) : (
        <BillingCard account={demoAccount ?? account} {...cardProps} />
      )}
      <ConfirmDialog
        open={planToConfirm !== null}
        onClose={() => {
          setPlanToConfirm(null);
          setChargeNowUpgrade(false);
        }}
        onConfirm={async () => {
          if (planToConfirm) await handleChangePlan(planToConfirm);
        }}
        title={MAX_UPGRADE_CONFIRM_TITLE}
        description={
          confirmError ??
          (chargeNowUpgrade ? MAX_UPGRADE_CHARGE_CONFIRM_BODY : MAX_UPGRADE_CONFIRM_BODY)
        }
        confirmLabel={MAX_UPGRADE_CONFIRM_LABEL}
        confirmBusyLabel={MAX_UPGRADE_BUSY_LABEL}
      />
    </section>
  );
}

type BillingCardProps = {
  account: AccountStatus;
  refreshing: boolean;
  spins: number;
  maxGrantPending: boolean;
  /** The pending grant wait outlasted its poll window (an abandoned or
   * still-open Stripe page). The plan stays suppressed, but the upgrade CTA
   * must come back: retrying opens a fresh hosted session and charges
   * nothing until the Stripe confirm. */
  maxGrantRetry: boolean;
  onRefresh: () => void;
  onUpgrade: (plan: SubscriptionPlan) => void;
  onChangePlan: (plan: SubscriptionPlan) => void;
  onManage: () => void;
};

function BillingCard({
  account,
  refreshing,
  spins,
  maxGrantPending,
  maxGrantRetry,
  onRefresh,
  onUpgrade,
  onChangePlan,
  onManage,
}: BillingCardProps) {
  const subscription = account.subscription;
  const liveSubscription = hasLiveSubscription(account);
  const usageRemainingPercent = usagePercentFromBalance(account.balance, subscription);
  const billingRecovery =
    subscription?.subscribed === true &&
    typeof subscription.status === "string" &&
    subscription.status.length > 0 &&
    !liveSubscription;
  // A subscribed account is on the paid plan even when the payload omits a
  // status (partial/older responses) — never relabel it as Free. billingRecovery
  // is a subset of this, kept to drive the "update billing" detail line.
  const onPaidPlan = subscription?.subscribed === true || liveSubscription;
  const canUpgrade = account.signedIn && !onPaidPlan;
  // Warm the meter toward "running low" so green never implies a near-empty
  // allowance. Only meaningful once we have a real signed-in reading.
  const lowUsage = account.signedIn && usageRemainingPercent <= 15;

  // Legacy subscription rows predate plan tiers and carry no slug; they are
  // all Pro, so anything that isn't explicitly "max" reads as Pro.
  // PATCH may optimistically mirror `plan: max` before payment and its credit
  // grant land. Keep the card on Pro until the grant poll confirms the credit balance
  // change, so "Max plan" and "Active" cannot leak early as a paired claim.
  const onMaxPlan = onPaidPlan && subscription?.plan === "max" && !maxGrantPending;
  const planName = onPaidPlan ? (onMaxPlan ? MAX_PLAN_NAME : PRO_PLAN_NAME) : FREE_PLAN_NAME;
  const planDetail = !onPaidPlan
    ? "No credit card required."
    : billingRecovery
      ? "Update billing in your account portal."
      : liveSubscription && subscription?.status === "trialing"
        ? (describeEnd("Billing starts", subscription.trialEnd) ?? "Free trial")
        : (describeEnd("Renews", subscription?.currentPeriodEnd) ?? "Active");
  const ctas: { label: string; onClick: () => void; title?: string }[] = onPaidPlan
    ? onMaxPlan || (maxGrantPending && !maxGrantRetry)
      ? [{ label: "Manage billing", onClick: onManage }]
      : // Pro subscribers keep billing management and can upgrade their
        // existing subscription in place; this is their path beyond Pro.
        // A slow grant wait keeps the retry path here too - the status line
        // points at trying again, so the affordance must exist.
        [
          { label: "Manage billing", onClick: onManage },
          {
            label: "Upgrade to Max",
            onClick: () => onChangePlan("max"),
            title: "For those who want to go beyond Pro",
          },
        ]
    : canUpgrade
      ? [
          { label: "Upgrade to Pro", onClick: () => onUpgrade("pro") },
          {
            label: "Upgrade to Max",
            onClick: () => onUpgrade("max"),
            title: "For those who want to go beyond Pro",
          },
        ]
      : [];

  return (
    <div className="settings-card billing-card">
      <div className="billing-plan">
        <div className="billing-plan-info">
          <h3 className="billing-plan-name">{planName}</h3>
          <p className="billing-plan-detail">{planDetail}</p>
        </div>
        {ctas.length > 0 ? (
          <div className="billing-plan-control">
            {ctas.map((cta) => (
              <button
                key={cta.label}
                type="button"
                className="btn btn-secondary"
                title={cta.title}
                onClick={cta.onClick}
              >
                {cta.label}
              </button>
            ))}
          </div>
        ) : null}
      </div>

      <div className="billing-usage">
        <div className="billing-usage-head">
          <span className="billing-usage-label">Usage remaining</span>
          <span className="billing-usage-right">
            <span className="billing-usage-value">{formatPercent(usageRemainingPercent)}</span>
            <button
              type="button"
              className="icon-button"
              aria-label="Refresh usage"
              title="Refresh usage"
              disabled={refreshing || !account.signedIn}
              onClick={onRefresh}
            >
              <IconArrowRotateClockwise
                size={14}
                className="balance-refresh-icon"
                style={{ transform: `rotate(${spins * 360}deg)` }}
              />
            </button>
          </span>
        </div>
        <div
          className={`usage-remaining-progress${lowUsage ? " is-low" : ""}`}
          role="progressbar"
          aria-label="Usage remaining"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={usageRemainingPercent}
        >
          <div
            className="usage-remaining-progress-fill"
            style={{
              transform: `scaleX(${usageRemainingPercent / 100})`,
            }}
          />
        </div>
      </div>
    </div>
  );
}

function displayName(account: AccountStatus) {
  return (
    account.user?.displayName ?? (account.user?.handle ? `@${account.user.handle}` : "Signed in")
  );
}

function formatPercent(percent: number) {
  return `${clampPercent(percent)}%`;
}

function clampPercent(percent: number) {
  if (!Number.isFinite(percent)) return 0;
  return Math.max(0, Math.min(100, Math.round(percent)));
}

export function usagePercentFromBalance(
  balance: AccountStatus["balance"],
  subscription: AccountStatus["subscription"],
) {
  if (Number.isFinite(balance?.usageRemainingPercent)) {
    return clampPercent(balance?.usageRemainingPercent ?? 0);
  }
  if (
    Number.isFinite(balance?.credits) &&
    Number.isFinite(subscription?.planCredits) &&
    (subscription?.planCredits ?? 0) > 0
  ) {
    return clampPercent(((balance?.credits ?? 0) / (subscription?.planCredits ?? 1)) * 100);
  }
  if (subscription?.subscribed === false && Number.isFinite(balance?.credits)) {
    return clampPercent(((balance?.credits ?? 0) / FREE_PLAN_CREDITS) * 100);
  }
  if (Number.isFinite(balance?.usdMillis)) {
    return (balance?.usdMillis ?? 0) > 0 ? 100 : 0;
  }
  return 0;
}

/** "Ends June 24" from an accounts-API timestamp, or undefined when the
 * date is missing or unparseable so callers can fall back to plain copy. */
function describeEnd(verb: string, timestamp?: string) {
  if (!timestamp) return undefined;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return undefined;
  // Annual plans renew up to a year out: include the year whenever the date
  // isn't in the current calendar year, so "Renews March 15" can't mean
  // either 3 or 15 months away.
  const showYear = date.getFullYear() !== new Date().getFullYear();
  const formatted = new Intl.DateTimeFormat(undefined, {
    month: "long",
    day: "numeric",
    ...(showYear ? { year: "numeric" } : {}),
  }).format(date);
  return `${verb} ${formatted}`;
}

function messageFromError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}
