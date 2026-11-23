import { IconDeepSearch } from "central-icons/IconDeepSearch";
import { IconEmail1Sparkle } from "central-icons/IconEmail1Sparkle";
import { IconFileSparkle } from "central-icons/IconFileSparkle";
import { IconHeartBeat } from "central-icons/IconHeartBeat";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconNotes } from "central-icons/IconNotes";
import { IconPageTextSearch } from "central-icons/IconPageTextSearch";
import { IconPieChart1 } from "central-icons/IconPieChart1";
import { IconShieldCheck } from "central-icons/IconShieldCheck";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import type { ReactNode } from "react";
import {
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_NEW_SESSION_PENDING_KEY,
  AGENT_SESSIONS_CHANGED_EVENT,
  AGENT_SESSION_RENAMED_EVENT,
  type AgentSessionRenamedDetail,
  type AgentSessionsChangedDetail,
} from "../../lib/agent-events";
import { type MessageKey, t } from "../../i18n";

export type AgentPanel = "chat" | "skills";

/**
 * The two write-access modes a new session can start the runtime in. The
 * sandbox is a kernel write-jail (reads are unrestricted either way), chosen
 * per new session — switching restarts Clovy's runtime, so the picker only
 * appears in the hero composer.
 */
// The Unrestricted confirm is a speed bump, not a recurring gate: one
// acknowledgment per app session, after which picking it arms directly.
// sessionStorage scopes that to the running app (a relaunch asks again) and
// survives the workspace remounting on view switches.
export const UNRESTRICTED_ACK_KEY = "clovy.agent.unrestrictedAcknowledged";

export function unrestrictedAcknowledged(): boolean {
  try {
    return window.sessionStorage.getItem(UNRESTRICTED_ACK_KEY) === "true";
  } catch {
    return false;
  }
}

export function rememberUnrestrictedAcknowledged() {
  try {
    window.sessionStorage.setItem(UNRESTRICTED_ACK_KEY, "true");
  } catch {
    // Ignore; worst case the dialog shows again.
  }
}

export const SANDBOX_OPTIONS = [
  {
    unrestricted: false,
    icon: <IconShieldCheck size={16} aria-hidden />,
    get title() {
      return t("agent.safety.sandboxed");
    },
    get description() {
      return t("agent.safety.sandboxedDescription");
    },
  },
  {
    unrestricted: true,
    icon: <IconShieldCrossed size={16} aria-hidden />,
    get title() {
      return t("agent.safety.unrestricted");
    },
    get description() {
      return t("agent.safety.unrestrictedDescription");
    },
  },
] as const;

export type AgentShortcut = {
  key: string;
  icon: ReactNode;
  title: string;
  description: string;
  prompt: string;
  /**
   * "prefill" drops the prompt into the composer for the user to finish; the
   * first `<placeholder>` token arrives as its bare phrase, selected for
   * overtyping — the angle brackets are authoring syntax and never reach the
   * composer. "attach" prefills and
   * opens the file picker. There is deliberately no action that submits on
   * click: every preset lands in the composer first, so the person sees
   * exactly what will run — and approves the spend — before it costs tokens.
   */
  action: "prefill" | "attach";
};

/**
 * Suggestion pool for the new-session hero. Shown HERO_SHORTCUT_COUNT at a
 * time and reshuffled on each visit, so the entry point stays a handful of
 * fresh ideas instead of a wall of ten cards. Pool order matters: the leading
 * window is the curated first-impression mix (a note-native ready-to-send
 * prompt, a placeholder prefill, an attach flow) that shows when the shuffle
 * is identity (e.g. in tests with Math.random mocked to 0). At least one
 * chip in that window should be something only Clovy can do — recapping your
 * own notes — not a generic computer chore.
 *
 * Every suggestion must succeed inside the default write-jail: reads are
 * broad, but writes land only in the agent workspace. Don't add shortcuts
 * that rename, move, or delete the user's files (tidy a folder, free up
 * disk space, dedupe) — the sandbox denies the write mid-task and Clovy's
 * own suggestion reads as broken.
 */
export const AGENT_SHORTCUTS: AgentShortcut[] = [
  {
    key: "recap-notes",
    icon: <IconNotes size={18} />,
    get title() {
      return t("agent.shortcut.recapNotes.title");
    },
    get description() {
      return t("agent.shortcut.recapNotes.description");
    },
    prompt:
      "Look through my recent meeting notes and give me a quick recap: what happened, what got decided, and any action items still open. Keep it brief.",
    action: "prefill",
  },
  {
    key: "research",
    icon: <IconDeepSearch size={18} />,
    get title() {
      return t("agent.shortcut.research.title");
    },
    get description() {
      return t("agent.shortcut.research.description");
    },
    prompt:
      "Research <a topic> and write a short summary (a few paragraphs) of what you find, with sources.",
    action: "prefill",
  },
  {
    key: "summarize-file",
    icon: <IconFileSparkle size={18} />,
    get title() {
      return t("agent.shortcut.summarizeFile.title");
    },
    get description() {
      return t("agent.shortcut.summarizeFile.description");
    },
    prompt: "Summarize the key points of the attached file and pull out any action items.",
    action: "attach",
  },
  {
    key: "health-check",
    icon: <IconHeartBeat size={18} />,
    get title() {
      return t("agent.shortcut.healthCheck.title");
    },
    get description() {
      return t("agent.shortcut.healthCheck.description");
    },
    prompt:
      "Give my computer a quick health check: free disk space, memory pressure, login items, and anything else worth flagging. Summarize what looks fine and what needs attention.",
    action: "prefill",
  },
  {
    key: "draft-follow-up",
    icon: <IconEmail1Sparkle size={18} />,
    get title() {
      return t("agent.shortcut.draftFollowUp.title");
    },
    get description() {
      return t("agent.shortcut.draftFollowUp.description");
    },
    prompt:
      "From my most recent meeting note, draft a short follow-up message covering the decisions and next steps.",
    action: "prefill",
  },
  {
    key: "find-file",
    icon: <IconMagnifyingGlass size={18} />,
    get title() {
      return t("agent.shortcut.findFile.title");
    },
    get description() {
      return t("agent.shortcut.findFile.description");
    },
    prompt:
      "Find <a file I half-remember> on my computer and tell me where it is. If several candidates match, list them with paths and dates.",
    action: "prefill",
  },
  {
    key: "analyze-spreadsheet",
    icon: <IconPieChart1 size={18} />,
    get title() {
      return t("agent.shortcut.analyzeSpreadsheet.title");
    },
    get description() {
      return t("agent.shortcut.analyzeSpreadsheet.description");
    },
    prompt:
      "Analyze the attached spreadsheet: summarize the key figures and trends, and call out anything that looks off.",
    action: "attach",
  },
  {
    key: "search-notes",
    icon: <IconPageTextSearch size={18} />,
    get title() {
      return t("agent.shortcut.searchNotes.title");
    },
    get description() {
      return t("agent.shortcut.searchNotes.description");
    },
    prompt:
      "Search my notes and transcripts for <what I'm trying to remember> and show me where it came up.",
    action: "prefill",
  },
];

/**
 * Hero greetings, one per visit: the heading cycles through this pool each
 * time the hero is entered, tracked in localStorage so the rotation continues
 * across launches. Exported so tests can match "any greeting". This is the
 * English source; `heroGreeting(index)` renders the entry in the interface
 * language.
 */
export const HERO_GREETINGS = [
  "What can Clovy do for you?",
  "What should we work on?",
  "Where should Clovy start?",
  "What can Clovy take off your plate?",
] as const;

const HERO_GREETING_KEYS: readonly MessageKey[] = [
  "agent.hero.greeting1",
  "agent.hero.greeting2",
  "agent.hero.greeting3",
  "agent.hero.greeting4",
];

export const HERO_GREETING_INDEX_KEY = "clovy:agent:hero-greeting";

/** Advance the per-visit rotation and return the pool index to show. */
export function advanceHeroGreetingIndex(): number {
  try {
    const index =
      Math.abs(
        Number.parseInt(window.localStorage.getItem(HERO_GREETING_INDEX_KEY) ?? "0", 10) || 0,
      ) % HERO_GREETINGS.length;
    window.localStorage.setItem(
      HERO_GREETING_INDEX_KEY,
      String((index + 1) % HERO_GREETINGS.length),
    );
    return index;
  } catch {
    // Storage unavailable: any greeting beats none.
    return Math.floor(Math.random() * HERO_GREETINGS.length);
  }
}

/** The greeting at `index` in the current interface language. */
export function heroGreeting(index: number): string {
  return t(HERO_GREETING_KEYS[index % HERO_GREETING_KEYS.length]);
}

// Three per hand so the row never wraps — a row-count jump mid-rotation would
// shove the footnote around every cycle.
export const HERO_SHORTCUT_COUNT = 3;
// Idle cadence for cycling the hand, and how long the cascade-out runs before
// the deck advances (300ms fade + 2 × 90ms stagger, see .agent-hero-chip).
export const HERO_ROTATE_MS = 8000;
export const HERO_CHIP_SWAP_MS = 500;

export {
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_NEW_SESSION_PENDING_KEY,
  AGENT_SESSIONS_CHANGED_EVENT,
  AGENT_SESSION_RENAMED_EVENT,
};

export type { AgentSessionRenamedDetail, AgentSessionsChangedDetail };
