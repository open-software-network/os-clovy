import { listen } from "@tauri-apps/api/event";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCircleQuestionmark } from "central-icons/IconCircleQuestionmark";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconStopCircle } from "central-icons/IconStopCircle";
import { createElement, type ComponentType, type SVGProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  AGENT_OPEN_EVENT,
  AGENT_RUN_SETTLED_EVENT,
  AGENT_SESSIONS_CHANGED_EVENT,
  AGENT_SESSION_STATUS_EVENT,
  type AgentRunSettledDetail,
  type AgentSessionStatusDetail,
  type AgentSessionStatusKind,
  type AgentSessionsChangedDetail,
} from "./lib/agent-events";
import {
  AGENT_HUD_ENABLED_KEY,
  AGENT_HUD_PLACEMENT_CHANGED_EVENT,
  AGENT_HUD_PLACEMENT_KEY,
  AGENT_HUD_VISIBILITY_CHANGED_EVENT,
  getAgentHudEnabled,
  getAgentHudPlacement,
  parseAgentHudPlacement,
  setAgentHudEnabled,
  type AgentHudPlacement,
  type AgentHudPlacementChangedDetail,
  type AgentHudVisibilityChangedDetail,
} from "./lib/agent-hud-settings";
import { isAgentSessionTitleCandidate, sessionSettledTitleKind } from "./lib/agent-session-titles";
import type { AgentSessionDto } from "./lib/agent-runtime-contract";
import { subscribeBrand } from "./lib/brand";
import { SPINNER_GRID_COLS, spinnerGrid } from "./lib/spinner-grid";
import { createHudLifecycle } from "./lib/hud-lifecycle";
import { titleFromPrompt } from "./lib/session-title";
import {
  agentHudHide,
  agentHudMainFocused,
  agentHudOpenAgent,
  agentHudSetLayout,
  agentHudShow,
} from "./lib/tauri";
import { installNativeContextMenuGuard } from "./lib/native-context-menu";
import { subscribeInterfaceLocale, subscribeInterfaceLocaleAcrossWindows, t } from "./i18n";
import "./styles/agent-hud.css";

const lifecycle = createHudLifecycle();

// Recolor this HUD window to the selected accent and keep it live-synced.
lifecycle.trackUnlisten(subscribeBrand());
// Follow the interface language chosen in the main window (sets <html lang>).
lifecycle.addCleanup(subscribeInterfaceLocaleAcrossWindows());

type HudSessionStatus = AgentSessionStatusKind | "idle";

type StatusRecord = AgentSessionStatusDetail & {
  receivedAt: number;
};

type HudEntry = {
  id: string;
  title: string;
  summary: string;
  status: HudSessionStatus;
  updatedAt: string;
  session?: AgentSessionDto;
  // Deep-link target for rows without a full session object (a status that
  // reported before, or without, a sessions-changed listing). The main
  // window resolves it against Clovy's stored session history.
  storedSessionId?: string;
};

const EXPANDED_KEY = "clovy:agent-hud:expanded";
// Emitted by the native panel (agent_hud.rs) when it swallows a right- or
// ctrl-click so the WKWebView never raises its own context menu. Keep this in
// sync with AGENT_HUD_CONTEXT_MENU_EVENT in agent_hud.rs.
const AGENT_HUD_CONTEXT_MENU_EVENT = "clovy:agent-hud:context-menu";
// Emitted by agent_hud.rs whenever the main window gains or loses focus.
// Keep in sync with AGENT_HUD_MAIN_FOCUS_EVENT there.
const AGENT_HUD_MAIN_FOCUS_EVENT = "clovy:agent-hud:main-focus";
const MAX_VISIBLE_ROWS = 3;
// Keep a finished session on screen long enough to actually read the "Done"
// row before it fades out, rather than blinking away the instant it lands.
const COMPLETED_STATUS_TTL_MS = 6500;
const FAILED_STATUS_TTL_MS = 8 * 1000;
// Covers the surface's fade-out transition (--t-slow) before the native
// window hides, plus a little slack for the compositor.
const WINDOW_FADE_MS = 300;
// How long the CSS collapse animation gets to play before the native
// window shrinks underneath it (--t-med plus slack).
const COLLAPSE_RESIZE_DELAY_MS = 200;

const hud = document.querySelector<HTMLElement>("#agent-hud");
const pill = document.querySelector<HTMLButtonElement>("#agent-hud-pill");
const mark = document.querySelector<HTMLElement>("#agent-hud-mark");
const pillLabel = document.querySelector<HTMLElement>("#agent-hud-pill-label");
const pillBadge = document.querySelector<HTMLElement>("#agent-hud-pill-badge");
const surface = document.querySelector<HTMLElement>(".agent-hud-surface");
const pillChevron = document.querySelector<HTMLElement>("#agent-hud-chevron");
const stack = document.querySelector<HTMLElement>("#agent-hud-stack");
const menu = document.querySelector<HTMLElement>("#agent-hud-menu");
const hideHud = document.querySelector<HTMLButtonElement>("#agent-hud-hide");

// The static page chrome from agent-hud.html, in the current language.
function applyStaticLabels() {
  document.title = t("hud.agents.windowTitle");
  surface?.setAttribute("aria-label", t("hud.agents.activity"));
  stack?.setAttribute("aria-label", t("hud.agents.sessions"));
  if (hideHud) hideHud.textContent = t("hud.agents.hide");
}

lifecycle.addCleanup(installNativeContextMenuGuard());

const state = {
  enabled: getAgentHudEnabled(),
  expanded: localStorage.getItem(EXPANDED_KEY) === "true",
  placement: getAgentHudPlacement(),
  // While the user is in Clovy itself the HUD stays down — the sidebar and
  // inline prompts already carry session state there. Rust streams focus
  // changes; the initial value is fetched below and defaults to "away" so
  // the standalone demo page still renders.
  mainFocused: false,
  focused: false,
  hovered: false,
  menuOpen: false,
  sessions: [] as AgentSessionDto[],
  workingSessionIds: new Set<string>(),
  waitingSessionIds: new Set<string>(),
  statusBySessionId: new Map<string, StatusRecord>(),
  promptBySessionId: new Map<string, string>(),
  pendingStatuses: [] as StatusRecord[],
  // Transient auto-expand triggered when an entry newly needs input. Unlike
  // `expanded` it is not persisted to EXPANDED_KEY: it grabs attention once,
  // then an explicit collapse (setExpanded(false)) must stick.
  attentionExpanded: false,
};

// Entry ids seen in `waitingForUser` on the previous render. A render that
// sees an id enter this set flips attentionExpanded on; the set is cleared
// when nothing is waiting so the next attention event pops the panel again.
let lastWaitingEntryIds = new Set<string>();

let lastLayoutKey = "";
let lastAppliedLayoutKey = "";
let lastStackKey = "";
let lastRenderedExpanded = false;
let lastWindowHeight = 0;
let cachedExpandedSurfaceWidth = 0;
let cachedExpandedPillHeight = 0;
let pruneTimer: number | undefined;
let hideTimer: number | undefined;
let resizeTimer: number | undefined;
let widthFlipTimer: number | undefined;
let windowShown = false;
let nativeVisibilityKnown = false;
let desiredWindowVisible = false;
let visibilityQueue: Promise<void> = Promise.resolve();
let layoutRequestId = 0;
let mainFocusEventRevision = 0;

lifecycle.addCleanup(() => {
  layoutRequestId += 1;
  window.clearTimeout(pruneTimer);
  window.clearTimeout(hideTimer);
  window.clearTimeout(resizeTimer);
  window.clearTimeout(widthFlipTimer);
});

function applySessionsChanged(detail?: AgentSessionsChangedDetail) {
  if (!detail) return;
  state.sessions = detail.sessions ?? [];
  state.workingSessionIds = new Set(detail.workingSessionIds ?? []);
  state.waitingSessionIds = new Set(detail.waitingSessionIds ?? []);
  const activeSessionIds = new Set([...state.workingSessionIds, ...state.waitingSessionIds]);
  const knownSessionIds = new Set(state.sessions.map((session) => session.id));
  for (const sessionId of state.promptBySessionId.keys()) {
    if (!knownSessionIds.has(sessionId) && !activeSessionIds.has(sessionId)) {
      state.promptBySessionId.delete(sessionId);
    }
  }
  for (const [sessionId, record] of state.statusBySessionId) {
    if (
      knownSessionIds.has(sessionId) &&
      isActiveStatus(record.status) &&
      !activeSessionIds.has(sessionId)
    ) {
      state.statusBySessionId.delete(sessionId);
    }
  }
  if (!activeSessionIds.size) {
    state.pendingStatuses = state.pendingStatuses.filter(
      (pending) => !isActiveStatus(pending.status),
    );
  }
  state.pendingStatuses = state.pendingStatuses.filter(
    (pending) => !state.sessions.some((session) => sameSubject(session, pending)),
  );
  render();
}

function applyStatus(detail?: AgentSessionStatusDetail) {
  if (!detail) return;
  const previous = detail.sessionId ? state.statusBySessionId.get(detail.sessionId) : undefined;
  const prompt =
    detail.prompt ??
    (detail.sessionId
      ? (previous?.prompt ?? state.promptBySessionId.get(detail.sessionId))
      : undefined);
  if (detail.sessionId && prompt) {
    state.promptBySessionId.set(detail.sessionId, prompt);
  }
  const record: StatusRecord = {
    ...detail,
    prompt,
    receivedAt: Date.now(),
  };
  if (detail.sessionId) {
    if (detail.status === "completed" || detail.status === "cancelled") {
      state.workingSessionIds.delete(detail.sessionId);
      state.waitingSessionIds.delete(detail.sessionId);
      state.statusBySessionId.set(detail.sessionId, terminalRecord(record));
      const replacedPending = replacePendingWithTerminalStatus(record);
      const hasKnownSession = state.sessions.some((session) => session.id === detail.sessionId);
      if (!hasKnownSession && !replacedPending) {
        state.pendingStatuses = [terminalRecord(record), ...state.pendingStatuses].slice(
          0,
          MAX_VISIBLE_ROWS,
        );
      }
      render();
      return;
    }
    state.statusBySessionId.set(detail.sessionId, record);
    state.pendingStatuses = state.pendingStatuses.filter(
      (pending) => !sameStatusSubject(pending, record),
    );
  } else {
    if (detail.status === "completed" || detail.status === "cancelled") {
      if (!replacePendingWithTerminalStatus(record)) {
        state.pendingStatuses = [terminalRecord(record), ...state.pendingStatuses].slice(
          0,
          MAX_VISIBLE_ROWS,
        );
      }
      render();
      return;
    }
    const key = statusSubject(record);
    state.pendingStatuses = [
      record,
      ...state.pendingStatuses.filter((item) => statusSubject(item) !== key),
    ].slice(0, MAX_VISIBLE_ROWS);
  }
  pruneOldStatuses();
  render();
}

function applyRunSettled(detail?: AgentRunSettledDetail) {
  if (!detail) return;
  applyStatus({
    ...detail,
    status: "completed",
  });
}

function applyVisibility(enabled: boolean) {
  state.enabled = enabled;
  if (!enabled) {
    state.focused = false;
    state.menuOpen = false;
  }
  render();
}

function applyPlacement(placement: AgentHudPlacement) {
  if (state.placement === placement) return;
  state.placement = placement;
  if (hud) hud.dataset.placement = placement;
  // Same content, new corner: force the layout call through so the native
  // window re-anchors even though nothing else in the key changed.
  lastLayoutKey = "";
  render();
}

function applyMainFocus(focused: boolean) {
  if (state.mainFocused === focused) return;
  state.mainFocused = focused;
  render();
}

/* The HUD has something to say AND the user is not already in Clovy looking
 * at the same state in richer form. Used by render() and syncWindowLayout(),
 * which must agree on what "visible" means. */
function hudVisible(hasEntries: boolean) {
  return state.enabled && hasEntries && !state.mainFocused;
}

function render() {
  if (!hud || !stack || !pill) return;

  pruneOldStatuses();
  const entries = buildEntries();
  const hasEntries = entries.length > 0;
  const hasAction = entries.some((entry) => entry.status === "waitingForUser");

  // Auto-expand only on the transition INTO waitingForUser: an id appearing
  // in the waiting set that wasn't there last render. A standing
  // waitingForUser must not keep forcing the panel open, or an explicit
  // collapse could never stick.
  const waitingEntryIds = new Set(
    entries.filter((entry) => entry.status === "waitingForUser").map((entry) => entry.id),
  );
  const newlyWaiting = [...waitingEntryIds].some((id) => !lastWaitingEntryIds.has(id));
  if (newlyWaiting) state.attentionExpanded = true;
  // Drop a stale flag once nothing is waiting, so it can't pop the panel
  // open again later.
  if (waitingEntryIds.size === 0) state.attentionExpanded = false;
  lastWaitingEntryIds = waitingEntryIds;

  const expanded =
    hudVisible(hasEntries) &&
    (state.attentionExpanded ||
      state.expanded ||
      state.focused ||
      // Hovering holds the panel open: it must not collapse or fade out
      // under the pointer, even when the reason it expanded goes away.
      (state.hovered && lastRenderedExpanded));
  lastRenderedExpanded = expanded;

  const visible = hudVisible(hasEntries);
  hud.dataset.hasEntries = hasEntries ? "true" : "false";
  hud.dataset.visible = visible ? "true" : "false";

  // Going invisible: leave the last-rendered content in place under the
  // CSS fade so the panel fades out showing "Done" instead of blanking
  // and vanishing. The native window hides once the fade has played.
  if (!visible && windowShown) {
    lastRenderedExpanded = false;
    void syncWindowLayout(false, 0, hasEntries);
    scheduleStatusPrune();
    return;
  }

  // The surface width comes from CSS (280px expanded, content-sized pill
  // collapsed) and `auto` cannot transition, so expand/collapse animates
  // it FLIP-style: measure before the state flips, re-measure after, run
  // the px-to-px transition.
  const willFlipWidth =
    surface !== null && windowShown && (hud.dataset.expanded === "true") !== expanded;
  const widthBefore = willFlipWidth ? surface.getBoundingClientRect().width : 0;

  hud.dataset.expanded = expanded ? "true" : "false";
  hud.dataset.hasAction = hasAction ? "true" : "false";
  hud.dataset.menuOpen = state.menuOpen ? "true" : "false";

  renderPill(entries, expanded);

  // Only rebuild the rows when their visible content changes. Status events
  // arrive in bursts while a session works; recreating identical nodes on
  // each one restarts CSS animations (the status spinner) and reads as
  // flicker. Rows stay in the DOM while collapsed so the expand and
  // collapse reveal always has content to animate.
  const stackKey = entries
    .map((entry) => [entry.id, entry.title, entry.summary, entry.status].join("\u0001"))
    .join("\u0002");
  if (stackKey !== lastStackKey) {
    lastStackKey = stackKey;
    stack.replaceChildren();
    entries.forEach((entry, index) => {
      stack.appendChild(renderRow(entry, index));
    });
  }
  stack.setAttribute("aria-hidden", expanded ? "false" : "true");
  if (menu) {
    menu.hidden = !state.menuOpen;
    menu.setAttribute("aria-hidden", state.menuOpen ? "false" : "true");
  }

  if (willFlipWidth && surface) flipSurfaceWidth(surface, widthBefore, expanded);

  void syncWindowLayout(expanded, expanded ? entries.length : 0, hasEntries);
  scheduleStatusPrune();
}

function flipSurfaceWidth(target: HTMLElement, fromWidth: number, expanded: boolean) {
  if (widthFlipTimer !== undefined) {
    window.clearTimeout(widthFlipTimer);
    widthFlipTimer = undefined;
  }
  target.style.width = "";
  // Reversing a collapse while its width transition is still running makes
  // getBoundingClientRect() report an in-between width. The native window
  // would then be parked at that transient width even after the CSS surface
  // finished expanding, clipping the left side of this right-anchored HUD.
  // Expanded width is a fixed CSS endpoint, so read that endpoint directly.
  const toWidth = expanded
    ? expandedSurfaceTargetWidth(target)
    : target.getBoundingClientRect().width;
  if (!fromWidth || !toWidth || Math.abs(toWidth - fromWidth) < 1) return;
  target.style.width = `${fromWidth}px`;
  // Force a layout so the starting width commits; the next assignment
  // then transitions instead of jumping.
  target.getBoundingClientRect();
  target.style.width = `${toWidth}px`;
  widthFlipTimer = window.setTimeout(() => {
    widthFlipTimer = undefined;
    // Hand the width back to the stylesheet (same computed value).
    target.style.width = "";
  }, 220);
}

function expandedSurfaceTargetWidth(target: HTMLElement) {
  if (cachedExpandedSurfaceWidth > 0) return cachedExpandedSurfaceWidth;
  const width = Number.parseFloat(
    window.getComputedStyle(target).getPropertyValue("--hud-expanded-w"),
  );
  if (Number.isFinite(width) && width > 0) {
    cachedExpandedSurfaceWidth = width;
  }
  return cachedExpandedSurfaceWidth;
}

function cssPixelToken(target: HTMLElement, token: string) {
  const value = Number.parseFloat(window.getComputedStyle(target).getPropertyValue(token));
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function expandedPillTargetHeight(target: HTMLElement) {
  if (cachedExpandedPillHeight > 0) return cachedExpandedPillHeight;
  const height = cssPixelToken(target, "--control-xl");
  if (height > 0) {
    cachedExpandedPillHeight = height;
  }
  return cachedExpandedPillHeight;
}

function menuEdgeOffset(target: HTMLElement) {
  const tokenOffset = expandedPillTargetHeight(target) + cssPixelToken(target, "--sp-3");
  // CSS is absent in the unit-test DOM. offsetTop is safe for top placements
  // and lets geometry-focused tests supply the rendered inset; production
  // reads tokens so bottom placement never enters the circular measurement.
  return tokenOffset > 0 ? tokenOffset : (menu?.offsetTop ?? 0);
}

function renderPill(entries: HudEntry[], expanded: boolean) {
  if (!pill || !mark || !pillLabel) return;
  const { label, status, runningCount, waitingCount } = pillSummary(entries);
  const activeCount = runningCount + waitingCount;
  const countOnly = status === "running" && runningCount > 1 && waitingCount === 0;
  mark.dataset.status = status;
  pill.dataset.countOnly = countOnly ? "true" : "false";
  pillLabel.textContent = label;
  if (pillBadge) {
    // Mixed state: the label leads with what needs the user; the badge
    // keeps the total active agent count visible at a glance.
    const showBadge = waitingCount > 0 && runningCount > 0;
    pillBadge.hidden = !showBadge;
    if (showBadge) {
      pillBadge.textContent = String(activeCount);
      const activeLabel = t("hud.agents.activeAgents", { count: activeCount });
      pillBadge.setAttribute("aria-label", activeLabel);
      pillBadge.title = activeLabel;
    }
  }
  pill.setAttribute("aria-expanded", expanded ? "true" : "false");
  pill.setAttribute("aria-label", expanded ? t("hud.agents.collapse") : t("hud.agents.expand"));
}

function pillSummary(entries: HudEntry[]): {
  label: string;
  status: HudSessionStatus;
  runningCount: number;
  waitingCount: number;
} {
  const waitingCount = entries.filter((entry) => entry.status === "waitingForUser").length;
  const runningCount = entries.filter(
    (entry) =>
      entry.status === "received" || entry.status === "starting" || entry.status === "running",
  ).length;
  if (waitingCount > 0) {
    return {
      label: t("hud.agents.needInput", { count: waitingCount }),
      status: "waitingForUser",
      runningCount,
      waitingCount,
    };
  }
  if (runningCount > 0) {
    return {
      label: t("hud.agents.running", { count: runningCount }),
      status: "running",
      runningCount,
      waitingCount,
    };
  }
  const [latest] = entries;
  if (latest) {
    return {
      label: statusLabel(latest.status),
      status: latest.status,
      runningCount,
      waitingCount,
    };
  }
  return { label: t("hud.status.idle"), status: "idle", runningCount, waitingCount };
}

function renderRow(entry: HudEntry, index: number) {
  const row = document.createElement("li");
  row.className = "agent-hud-row";
  row.dataset.status = entry.status;
  // Staggers the rows' fade-in when the panel expands (CSS transition
  // delay). Rows created while already expanded paint directly in their
  // final state, so they never re-run the entrance.
  row.style.setProperty("--row-index", String(index));

  const body = document.createElement("button");
  body.type = "button";
  body.className = "agent-hud-row-body";
  body.addEventListener("click", () => {
    void openAgent(entry.session, entry.storedSessionId);
  });

  const status = document.createElement("span");
  status.className = "agent-hud-status";
  status.dataset.status = entry.status;
  status.setAttribute("aria-hidden", "true");
  appendStatusIcon(status, entry.status);
  body.appendChild(status);

  const text = document.createElement("span");
  text.className = "agent-hud-row-text";

  const title = document.createElement("span");
  title.className = "agent-hud-row-title";
  title.textContent = entry.title;
  text.appendChild(title);

  const summaryText = rowSummary(entry);
  if (summaryText) {
    const summary = document.createElement("span");
    summary.className = "agent-hud-row-summary";
    summary.textContent = summaryText;
    text.appendChild(summary);
  }

  body.appendChild(text);
  row.appendChild(body);

  return row;
}

function buildEntries() {
  const now = Date.now();
  const entries: HudEntry[] = [];
  const seen = new Set<string>();

  for (const session of state.sessions) {
    const id = session.id;
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const record = state.statusBySessionId.get(id);
    if (record && isExpiredTerminalRecord(record, now)) {
      state.statusBySessionId.delete(id);
    }
    const entry = entryFromSession(session, state.statusBySessionId.get(id));
    if (shouldRenderEntry(entry)) entries.push(entry);
  }

  for (const record of state.pendingStatuses) {
    const entry = entryFromPending(record);
    if (shouldRenderEntry(entry)) entries.push(entry);
  }

  return entries.sort(compareEntries).slice(0, MAX_VISIBLE_ROWS);
}

function entryFromSession(session: AgentSessionDto, record?: StatusRecord): HudEntry {
  const status = sessionStatus(session, record);
  return {
    id: session.id,
    title: sessionTitle(session, record),
    summary: sessionSummary(status, record),
    status,
    updatedAt: sessionTimestamp(session, record),
    session,
  };
}

function entryFromPending(record: StatusRecord): HudEntry {
  return {
    id: `pending:${statusSubject(record)}`,
    title: statusTitle(record),
    summary: statusSummary(record),
    status: record.status,
    updatedAt: new Date(record.receivedAt).toISOString(),
    storedSessionId: record.sessionId,
  };
}

function sessionStatus(session: AgentSessionDto, record?: StatusRecord): HudSessionStatus {
  if (record && isTerminalStatus(record.status) && !isExpiredTerminalRecord(record)) {
    return record.status;
  }
  if (state.waitingSessionIds.has(session.id)) return "waitingForUser";
  if (state.workingSessionIds.has(session.id)) return "running";
  if (record && isActiveStatus(record.status)) {
    return record.status;
  }
  return "idle";
}

function sessionTitle(session: AgentSessionDto, record?: StatusRecord) {
  const storedTitle = session.title?.trim();
  // Runtime-owned session titles are persisted application data. When there
  // is no newer lifecycle title to reconcile, keep them verbatim, including
  // intentional question-shaped and manually entered titles.
  if (storedTitle && !record?.title?.trim()) return storedTitle;
  const settledTitleKind = sessionSettledTitleKind(session.id);
  const storedTitleIsAuthoritative =
    storedTitle &&
    (settledTitleKind || !isPromptDerivedTitle(storedTitle, record?.prompt)) &&
    isHudStoredSessionTitleSafe(storedTitle, session.id, record?.prompt, record?.prompt);
  if (storedTitleIsAuthoritative) {
    return storedTitle;
  }
  const recordTitle = record?.title?.trim();
  if (recordTitle) {
    if (isHudStatusTitleSafe(recordTitle, session.id, record?.prompt)) return recordTitle;
    const safeSessionTitle = storedTitle;
    if (
      safeSessionTitle &&
      isHudStoredSessionTitleSafe(safeSessionTitle, session.id, record?.prompt, record?.prompt)
    ) {
      return safeSessionTitle;
    }
    return record?.prompt?.trim() || t("hud.agents.untitledSession");
  }
  if (storedTitle) {
    return isHudStoredSessionTitleSafe(storedTitle, session.id, record?.prompt, record?.prompt)
      ? storedTitle
      : record?.prompt?.trim() || t("hud.agents.untitledSession");
  }
  return record?.prompt?.trim() || t("hud.agents.untitledSession");
}

function sessionSummary(status: HudSessionStatus, record?: StatusRecord) {
  const summary = record?.summary?.trim();
  if (summary) return summary;
  if (status !== "idle") return statusLabel(status);
  return t("hud.status.idle");
}

function sessionTimestamp(session: AgentSessionDto, record?: StatusRecord) {
  if (record) return new Date(record.receivedAt).toISOString();
  return session.updatedAt;
}

function statusTitle(record: StatusRecord) {
  const title = record.title?.trim();
  return title && isHudStatusTitleSafe(title, record.sessionId, record.prompt)
    ? title
    : record.prompt?.trim() || t("hud.agents.untitledSession");
}

function isHudStatusTitleSafe(title: string, sessionId?: string, prompt?: string) {
  const settledTitleKind = sessionSettledTitleKind(sessionId);
  if (settledTitleKind === "manual") return true;
  if (settledTitleKind === "exchange") return isAgentSessionTitleCandidate(title);
  return isPromptDerivedTitle(title, prompt) || isAgentSessionTitleCandidate(title);
}

function isHudStoredSessionTitleSafe(
  title: string,
  sessionId: string,
  ...promptCandidates: Array<string | undefined>
) {
  const settledTitleKind = sessionSettledTitleKind(sessionId);
  if (settledTitleKind === "manual") return true;
  if (settledTitleKind === "exchange") return isAgentSessionTitleCandidate(title);
  return (
    promptCandidates.some((prompt) => isPromptDerivedTitle(title, prompt)) ||
    isAgentSessionTitleCandidate(title)
  );
}

function isPromptDerivedTitle(title: string, prompt?: string) {
  return Boolean(prompt?.trim() && titleFromPrompt(prompt) === title);
}

function statusSummary(record: StatusRecord) {
  return record.summary?.trim() || statusLabel(record.status);
}

function rowSummary(entry: HudEntry) {
  const summary = entry.summary.trim();
  if (!summary) return undefined;

  const normalizedSummary = normalizeText(summary);
  if (
    normalizedSummary === normalizeText(entry.title) ||
    normalizedSummary === normalizeText(statusLabel(entry.status)) ||
    normalizedSummary === "june is working" ||
    normalizedSummary === "clovy is working" ||
    normalizedSummary === "starting june" ||
    normalizedSummary === "starting clovy" ||
    normalizedSummary === "june finished" ||
    normalizedSummary === "clovy finished"
  ) {
    return undefined;
  }

  return summary;
}

function normalizeText(value: string) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[.!?]+$/g, "");
}

function statusLabel(status: HudSessionStatus) {
  switch (status) {
    case "received":
    case "starting":
    case "running":
      return t("hud.agents.status.thinking");
    case "waitingForUser":
      return t("hud.agents.status.needsInput");
    case "completed":
      return t("hud.agents.status.done");
    case "failed":
      return t("hud.agents.status.failed");
    case "cancelled":
      return t("hud.agents.status.stopped");
    case "idle":
      return t("hud.status.idle");
    default:
      return t("hud.status.idle");
  }
}

function compareEntries(a: HudEntry, b: HudEntry) {
  const rank = statusRank(a.status) - statusRank(b.status);
  if (rank !== 0) return rank;
  return (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
}

function statusRank(status: HudSessionStatus) {
  if (status === "waitingForUser") return 0;
  if (status === "received" || status === "starting" || status === "running") return 1;
  if (status === "failed") return 2;
  if (status === "completed" || status === "cancelled") return 3;
  return 4;
}

function isActiveStatus(status: HudSessionStatus) {
  return (
    status === "received" ||
    status === "starting" ||
    status === "running" ||
    status === "waitingForUser"
  );
}

function pruneOldStatuses() {
  const now = Date.now();
  state.pendingStatuses = state.pendingStatuses.filter(
    (record) =>
      isActiveStatus(record.status) ||
      (isTerminalStatus(record.status) && !isExpiredTerminalRecord(record, now)),
  );
  for (const [id, record] of state.statusBySessionId) {
    if (isExpiredTerminalRecord(record, now)) {
      state.statusBySessionId.delete(id);
    }
  }
}

function replacePendingWithTerminalStatus(record: StatusRecord) {
  let replaced = false;
  state.pendingStatuses = state.pendingStatuses.map((item) => {
    if (!sameStatusSubject(item, record)) return item;
    replaced = true;
    return terminalRecord(record, item);
  });
  if (record.activeCount === 0) {
    const activePending = state.pendingStatuses.filter((item) => isActiveStatus(item.status));
    // The status stream says no active work remains. Mark every still-active
    // anonymous row terminal even when one row already matched by subject;
    // each row keeps its own title, while the terminal summary comes from this
    // final record and may describe only the last session that reported.
    state.pendingStatuses = [
      ...activePending.map((item) => terminalRecord(record, item)),
      ...state.pendingStatuses.filter((item) => !isActiveStatus(item.status)),
    ].slice(0, MAX_VISIBLE_ROWS);
    return replaced || activePending.length > 0;
  }
  if (replaced) return true;
  const activePending = state.pendingStatuses.filter((item) => isActiveStatus(item.status));
  if (activePending.length === 1) {
    state.pendingStatuses = state.pendingStatuses.map((item) =>
      item === activePending[0] ? terminalRecord(record, item) : item,
    );
    return true;
  }
  return false;
}

function terminalRecord(record: StatusRecord, previous?: StatusRecord) {
  return {
    ...record,
    prompt: previous?.prompt ?? record.prompt,
    title: previous?.title ?? record.title,
    // Left empty when the runtime sent no summary so the row falls back to
    // the status label at render time, in the current interface language.
    summary: record.summary?.trim() || undefined,
    receivedAt: record.receivedAt,
  };
}

function scheduleStatusPrune() {
  if (pruneTimer !== undefined) {
    window.clearTimeout(pruneTimer);
    pruneTimer = undefined;
  }
  // Expiry is paused while hovered; the pointerleave render reschedules.
  if (state.hovered) return;
  const now = Date.now();
  const expirations = [...state.pendingStatuses, ...Array.from(state.statusBySessionId.values())]
    .map((record) => terminalExpiration(record))
    .filter((expiration): expiration is number => expiration !== undefined);
  if (!expirations.length) return;
  const delay = Math.max(0, Math.min(...expirations) - now) + 25;
  pruneTimer = window.setTimeout(() => {
    pruneTimer = undefined;
    pruneOldStatuses();
    render();
  }, delay);
}

function terminalExpiration(record: StatusRecord) {
  const ttl = terminalStatusTtl(record.status);
  return ttl === undefined ? undefined : record.receivedAt + ttl;
}

function isExpiredTerminalRecord(record: StatusRecord, now = Date.now()) {
  // Terminal rows never expire under the pointer; the user is reading them.
  if (state.hovered) return false;
  const expiration = terminalExpiration(record);
  return expiration !== undefined && now > expiration;
}

function terminalStatusTtl(status: HudSessionStatus) {
  if (status === "completed" || status === "cancelled") {
    return COMPLETED_STATUS_TTL_MS;
  }
  if (status === "failed") return FAILED_STATUS_TTL_MS;
  return undefined;
}

function shouldRenderEntry(entry: HudEntry) {
  return isActiveStatus(entry.status) || isTerminalStatus(entry.status);
}

function isTerminalStatus(status: HudSessionStatus) {
  return status === "completed" || status === "cancelled" || status === "failed";
}

function sameSubject(session: AgentSessionDto, record: StatusRecord) {
  const title = statusSubject(record);
  return session.id === record.sessionId || session.title?.trim().toLowerCase() === title;
}

function sameStatusSubject(a: StatusRecord, b: StatusRecord) {
  return statusSubject(a) === statusSubject(b);
}

function statusSubject(record: StatusRecord) {
  return (record.title?.trim() || record.prompt?.trim() || "Agent session").toLowerCase();
}

async function syncWindowLayout(expanded: boolean, rowCount: number, hasEntries: boolean) {
  const menuOpen = state.menuOpen;
  const visible = hudVisible(hasEntries);
  const bounds = visibleHudBounds();
  const key = `${visible}:${expanded}:${rowCount}:${menuOpen}:${state.placement}:${bounds.width ?? 0}:${bounds.height ?? 0}`;
  if (key === lastLayoutKey) return;
  lastLayoutKey = key;
  const requestId = ++layoutRequestId;
  if (!visible) {
    cancelPendingResize();
    scheduleWindowHide(!state.enabled);
    return;
  }
  cancelWindowHide();
  cancelPendingResize();
  const height = bounds.height ?? nativeWindowHeight(expanded, rowCount, menuOpen);
  const apply = async (latestBounds: { width?: number; height?: number }) => {
    if (lifecycle.signal.aborted || requestId !== layoutRequestId || !hudVisible(hasEntries)) {
      return;
    }
    const appliedKey = `${expanded}:${rowCount}:${menuOpen}:${state.placement}:${latestBounds.width ?? 0}:${latestBounds.height ?? 0}`;
    const appliedHeight = latestBounds.height ?? nativeWindowHeight(expanded, rowCount, menuOpen);
    if (appliedKey !== lastAppliedLayoutKey) {
      // Record the request before awaiting IPC. Rapid event bursts can render
      // again while Tauri is still resolving the first call; treating the
      // requested layout as current prevents redundant native resize and
      // re-position work for the same endpoint.
      lastAppliedLayoutKey = appliedKey;
      lastWindowHeight = appliedHeight;
      await agentHudSetLayout({
        expanded,
        cardCount: rowCount,
        placement: state.placement,
        ...(menuOpen ? { contextMenuOpen: menuOpen } : {}),
        ...latestBounds,
      }).catch(() => {
        if (lastAppliedLayoutKey === appliedKey) lastAppliedLayoutKey = "";
      });
    }
    if (lifecycle.signal.aborted || requestId !== layoutRequestId || !hudVisible(hasEntries)) {
      return;
    }
    await setNativeWindowVisible(true);
  };
  // Growing: the window must be at full size before the CSS reveal plays.
  // Shrinking: the reveal collapses first, then the window snaps down under
  // the (already pill-sized) surface; resizing immediately would clip the
  // animation mid-flight.
  if (windowShown && height < lastWindowHeight) {
    resizeTimer = window.setTimeout(() => {
      resizeTimer = undefined;
      void apply(visibleHudBounds());
    }, COLLAPSE_RESIZE_DELAY_MS);
  } else {
    // Immediate growth has no intervening animation frame, so the bounds
    // already measured for the key are still current. Reuse them instead of
    // forcing a second style/layout pass on every render.
    await apply(bounds);
  }
}

function visibleHudBounds(): { width?: number; height?: number } {
  if (!surface) return {};
  if (state.menuOpen && (!menu || menu.offsetWidth <= 0 || menu.offsetHeight <= 0)) return {};
  // The last row's tokenized bottom margin lives inside scrollable overflow
  // that scrollHeight does not reliably include. Add it explicitly so the
  // panel's bottom corner cannot paint clipped during the first reveal.
  const stackBottomGap = cssPixelToken(surface, "--sp-1");
  const measuredSurfaceWidth = surface.offsetWidth;
  const expandedStackHeight = hud?.dataset.expanded === "true" && stack ? stack.scrollHeight : 0;
  const expandedSurfaceHeight =
    expandedStackHeight > 0 && pill
      ? Math.max(expandedPillTargetHeight(surface), pill.offsetHeight) +
        expandedStackHeight +
        stackBottomGap
      : surface.offsetHeight;
  // grid-template-rows animates the reveal from 0fr to 1fr. scrollHeight
  // exposes the final row height even while offsetHeight is still mid-
  // transition, so the native window grows before the reveal can be clipped.
  const surfaceHeight = Math.max(surface.offsetHeight, expandedSurfaceHeight);
  // FLIP writes the final expanded width inline before the transition starts.
  // offsetWidth reports an in-flight value during that transition, so prefer
  // the known target to keep the native window from clipping the reveal.
  const animatedExpandedWidth =
    hud?.dataset.expanded === "true" ? Number.parseFloat(surface.style.width) : 0;
  const expandedTargetWidth =
    hud?.dataset.expanded === "true" ? expandedSurfaceTargetWidth(surface) : 0;
  const surfaceWidth = Math.max(
    measuredSurfaceWidth,
    Number.isFinite(animatedExpandedWidth) ? animatedExpandedWidth : 0,
    expandedTargetWidth,
  );
  const width = Math.max(surfaceWidth, state.menuOpen && menu ? menu.offsetWidth : 0);
  // Intrinsic extents, not offsetTop-based ones: with a bottom-anchored
  // placement the surface's offsetTop tracks the current window height, and
  // sizing the window from it can never shrink (the measurement is circular).
  const height = Math.max(
    surfaceHeight,
    state.menuOpen && menu ? menuEdgeOffset(surface) + menu.offsetHeight : 0,
  );
  // A CSS endpoint alone is not a rendered bound. During jsdom/startup the
  // custom property is readable before the surface itself has dimensions;
  // fall back to Rust's startup geometry until layout has actually happened.
  if (measuredSurfaceWidth <= 0 || width <= 0 || height <= stackBottomGap) return {};
  return { width: Math.ceil(width), height: Math.ceil(height) };
}

/* Mirrors agent_hud_window_size in agent_hud.rs, only to tell growth from
 * shrinkage; the Rust side stays the source of truth for the real size. */
function nativeWindowHeight(expanded: boolean, rowCount: number, menuOpen: boolean) {
  const height = !expanded || rowCount === 0 ? 58 : 8 + 36 + Math.min(rowCount, 3) * 46 + 6 + 14;
  return menuOpen ? Math.max(height, 104) : height;
}

function cancelPendingResize() {
  if (resizeTimer === undefined) return;
  window.clearTimeout(resizeTimer);
  resizeTimer = undefined;
}

function scheduleWindowHide(immediate = false) {
  cancelWindowHide();
  if (!windowShown || immediate) {
    void setNativeWindowVisible(false);
    return;
  }
  hideTimer = window.setTimeout(() => {
    hideTimer = undefined;
    void setNativeWindowVisible(false);
  }, WINDOW_FADE_MS);
}

function cancelWindowHide() {
  if (hideTimer === undefined) return;
  window.clearTimeout(hideTimer);
  hideTimer = undefined;
}

function setNativeWindowVisible(visible: boolean) {
  desiredWindowVisible = visible;
  const reconcile = visibilityQueue.then(async () => {
    if (
      lifecycle.signal.aborted ||
      (nativeVisibilityKnown && desiredWindowVisible === windowShown)
    ) {
      return;
    }
    const showing = desiredWindowVisible;
    try {
      if (showing) {
        await agentHudShow();
      } else {
        await agentHudHide();
      }
    } catch {
      return;
    }
    if (lifecycle.signal.aborted) return;
    windowShown = showing;
    nativeVisibilityKnown = true;
    if (!showing) {
      // A later show should re-fit and re-anchor in case fonts, placement, or
      // monitor geometry changed while the panel was hidden.
      lastAppliedLayoutKey = "";
    }
  });
  visibilityQueue = reconcile.catch(() => {});
  return reconcile;
}

function setExpanded(expanded: boolean) {
  if (!expanded) {
    state.focused = false;
    state.menuOpen = false;
    // An explicit collapse clears the attention auto-expand; otherwise the
    // next render would immediately re-expand while a session still waits.
    state.attentionExpanded = false;
    // An explicit collapse beats the hover-hold; the pointer is necessarily
    // over the pill when it is clicked.
    lastRenderedExpanded = false;
  }
  state.expanded = expanded;
  localStorage.setItem(EXPANDED_KEY, expanded ? "true" : "false");
  render();
}

type CentralIcon = ComponentType<
  SVGProps<SVGSVGElement> & { size?: string | number; ariaHidden?: boolean }
>;

function appendIcon(parent: HTMLElement, Icon: CentralIcon, size: number) {
  const wrapper = document.createElement("span");
  wrapper.className = "agent-hud-icon";
  wrapper.setAttribute("aria-hidden", "true");
  wrapper.innerHTML = renderToStaticMarkup(
    createElement(Icon, {
      size,
      ariaHidden: true,
      focusable: false,
    }),
  );
  parent.appendChild(wrapper);
}

function setIcon(parent: HTMLElement | null, Icon: CentralIcon, size: number) {
  if (!parent) return;
  parent.replaceChildren();
  appendIcon(parent, Icon, size);
}

function appendStatusIcon(parent: HTMLElement, status: HudSessionStatus) {
  switch (status) {
    case "waitingForUser":
      appendIcon(parent, IconCircleQuestionmark, 14);
      return;
    case "completed":
      appendIcon(parent, IconCheckmark2Small, 14);
      return;
    case "failed":
    case "cancelled":
      appendIcon(parent, status === "failed" ? IconCrossSmall : IconStopCircle, 14);
      return;
    case "received":
    case "starting":
    case "running":
      appendDotSpinner(parent);
      return;
    case "idle":
      return;
  }
}

// The app-wide neutral dot spinner (see components/DotSpinner.tsx
// and lib/spinner-grid); this page has no React tree, so the same markup is
// built by hand against the shared dot-spinner.css. Uses the sm (3×3) variant,
// matching every inline usage.
function appendDotSpinner(parent: HTMLElement) {
  const spinner = document.createElement("span");
  spinner.className = "dot-spinner";
  spinner.dataset.size = "sm";
  spinner.setAttribute("aria-hidden", "true");
  spinner.style.setProperty("--spinner-cols", String(SPINNER_GRID_COLS.sm));
  for (const cell of spinnerGrid("sm")) {
    const dot = document.createElement("span");
    dot.style.setProperty("--spinner-order", String(cell.order));
    if (cell.mark) {
      dot.dataset.mark = "";
    }
    spinner.appendChild(dot);
  }
  parent.appendChild(spinner);
}

async function openAgent(session?: AgentSessionDto, storedSessionId?: string) {
  await agentHudOpenAgent(session, storedSessionId).catch(() => {
    window.dispatchEvent(
      new CustomEvent(AGENT_OPEN_EVENT, {
        detail: { session, storedSessionId },
      }),
    );
  });
}

function toggleExpanded() {
  const renderedExpanded = hud?.dataset.expanded === "true";
  setExpanded(!renderedExpanded);
}

function setFocusExpanded(focused: boolean) {
  const changed = state.focused !== focused;
  state.focused = focused;
  let menuClosed = false;
  if (!focused && state.menuOpen) {
    state.menuOpen = false;
    menuClosed = true;
  }
  if (changed || menuClosed) render();
}

function openMenu() {
  state.menuOpen = true;
  render();
  window.setTimeout(() => hideHud?.focus(), 0);
}

function closeMenu() {
  if (!state.menuOpen) return;
  state.menuOpen = false;
  render();
}

function hideFromMenu() {
  closeMenu();
  setAgentHudEnabled(false);
}

function setHovered(hovered: boolean) {
  if (state.hovered === hovered) return;
  if (!hovered) {
    // Records that expired while held under the pointer restart their TTL,
    // so rows linger briefly instead of vanishing the instant it leaves.
    const now = Date.now();
    const records = [...state.pendingStatuses, ...state.statusBySessionId.values()];
    for (const record of records) {
      const expiration = terminalExpiration(record);
      if (expiration !== undefined && now > expiration) {
        record.receivedAt = now;
      }
    }
  }
  state.hovered = hovered;
  render();
}

hud?.addEventListener("pointerenter", () => {
  setHovered(true);
});

hud?.addEventListener("pointerleave", () => {
  if (state.menuOpen) closeMenu();
  setHovered(false);
});

hud?.addEventListener("focusin", () => {
  setFocusExpanded(true);
});

hud?.addEventListener("focusout", (event) => {
  const next = event.relatedTarget;
  if (next instanceof Node && hud.contains(next)) return;
  setFocusExpanded(false);
});

pill?.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  event.stopPropagation();
  toggleExpanded();
});

pill?.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  toggleExpanded();
});

// The Agent HUD has no text-selection use case, so suppress the native
// WKWebView context menu everywhere in this window and surface our
// own menu instead. In the real app the native panel swallows right- and
// ctrl-clicks before WKWebView sees them (see the Tauri listener below);
// this DOM listener is the fallback for the standalone browser/demo page,
// where there is no native panel to intercept.
window.addEventListener(
  "contextmenu",
  (event) => {
    event.preventDefault();
    event.stopPropagation();
    openMenu();
  },
  { signal: lifecycle.signal },
);

menu?.addEventListener("pointerdown", (event) => {
  event.stopPropagation();
});

hideHud?.addEventListener("click", (event) => {
  event.preventDefault();
  event.stopPropagation();
  hideFromMenu();
});

window.addEventListener(
  "pointerdown",
  (event) => {
    if (!state.menuOpen) return;
    const target = event.target;
    if (target instanceof Node && menu?.contains(target)) return;
    closeMenu();
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  "keydown",
  (event) => {
    if (event.key === "Escape") closeMenu();
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  AGENT_HUD_VISIBILITY_CHANGED_EVENT,
  (event) => {
    const detail = (event as CustomEvent<AgentHudVisibilityChangedDetail>).detail;
    if (detail) applyVisibility(detail.enabled);
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  AGENT_SESSIONS_CHANGED_EVENT,
  (event) => {
    applySessionsChanged((event as CustomEvent<AgentSessionsChangedDetail>).detail);
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  AGENT_SESSION_STATUS_EVENT,
  (event) => {
    applyStatus((event as CustomEvent<AgentSessionStatusDetail>).detail);
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  AGENT_RUN_SETTLED_EVENT,
  (event) => {
    applyRunSettled((event as CustomEvent<AgentRunSettledDetail>).detail);
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  AGENT_HUD_PLACEMENT_CHANGED_EVENT,
  (event) => {
    const detail = (event as CustomEvent<AgentHudPlacementChangedDetail>).detail;
    if (detail) applyPlacement(detail.placement);
  },
  { signal: lifecycle.signal },
);

window.addEventListener(
  "storage",
  (event) => {
    if (event.key === AGENT_HUD_ENABLED_KEY) {
      applyVisibility(event.newValue !== "false");
    }
    if (event.key === AGENT_HUD_PLACEMENT_KEY) {
      applyPlacement(parseAgentHudPlacement(event.newValue));
    }
  },
  { signal: lifecycle.signal },
);

lifecycle.trackUnlisten(
  listen<AgentSessionsChangedDetail>(AGENT_SESSIONS_CHANGED_EVENT, (event) =>
    applySessionsChanged(event.payload),
  ),
);

lifecycle.trackUnlisten(
  listen<AgentSessionStatusDetail>(AGENT_SESSION_STATUS_EVENT, (event) =>
    applyStatus(event.payload),
  ),
);

lifecycle.trackUnlisten(
  listen<AgentRunSettledDetail>(AGENT_RUN_SETTLED_EVENT, (event) => applyRunSettled(event.payload)),
);

lifecycle.trackUnlisten(
  listen<AgentHudVisibilityChangedDetail>(AGENT_HUD_VISIBILITY_CHANGED_EVENT, (event) =>
    applyVisibility(event.payload.enabled),
  ),
);

lifecycle.trackUnlisten(
  listen<AgentHudPlacementChangedDetail>(AGENT_HUD_PLACEMENT_CHANGED_EVENT, (event) =>
    applyPlacement(event.payload.placement),
  ),
);

const mainFocusListenerRegistration = listen<boolean>(AGENT_HUD_MAIN_FOCUS_EVENT, (event) => {
  mainFocusEventRevision += 1;
  applyMainFocus(Boolean(event.payload));
});
lifecycle.trackUnlisten(mainFocusListenerRegistration);

// Subscribe before sampling so a focus transition during listener setup is
// reflected by the later snapshot. Events received while that snapshot is in
// flight advance the revision and take precedence over its result.
void mainFocusListenerRegistration
  .then(async () => {
    if (lifecycle.signal.aborted) return;
    const initialFocusEventRevision = mainFocusEventRevision;
    const focused = await agentHudMainFocused();
    if (!lifecycle.signal.aborted && mainFocusEventRevision === initialFocusEventRevision) {
      applyMainFocus(Boolean(focused));
    }
  })
  .catch(() => {});

// The native panel intercepts the right-/ctrl-click and asks us to open the
// menu. The click never reaches the DOM, so there is no competing
// pointerdown to close it again (the window pointerdown handler only fires
// for clicks the webview actually receives).
lifecycle.trackUnlisten(listen(AGENT_HUD_CONTEXT_MENU_EVENT, () => openMenu()));

// First-load metrics race: the window can be sized while ABC Diatype is
// still loading, and the reflow when the faces land grows the rows past the
// measured frame. Re-fit once fonts settle (same companion the dictation
// HUD keeps in hud.ts).
if (typeof document.fonts?.ready?.then === "function") {
  void document.fonts.ready.then(() => {
    if (lifecycle.signal.aborted) return;
    lastLayoutKey = "";
    render();
  });
}

// Language change from the main window: relabel the chrome and re-render the
// rows (their text is part of the stack key, so they rebuild) and re-fit the
// native window to the new label widths.
lifecycle.addCleanup(
  subscribeInterfaceLocale(() => {
    applyStaticLabels();
    lastLayoutKey = "";
    render();
  }),
);

if (hud) hud.dataset.placement = state.placement;
setIcon(pillChevron, IconChevronDownSmall, 14);
applyStaticLabels();
render();

// Console driver for this page when served standalone in a browser:
// __agentHud("waiting") etc. See lib/agent-hud-demo.ts.
if (import.meta.env.DEV) {
  void import("./lib/agent-hud-demo").then(({ registerAgentHudDemo }) => {
    if (!lifecycle.signal.aborted) registerAgentHudDemo({ local: true });
  });
}
