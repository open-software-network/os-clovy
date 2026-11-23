import { formatClockTime, formatDate, intlLocale, t } from "../i18n/translate";

export const CLOVY_HOME_SESSION_IDS_STORAGE_KEY = "clovy:home:session-ids:v1";
export const CLOVY_HOME_CHECK_INS_STORAGE_KEY = "clovy:home:check-ins:v1";
export const CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY = "clovy:home:task-handoffs:v1";
export const CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY = "clovy:home:direct-turns:v1";
export const LEGACY_CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY = "clovy.home.taskHandoffs.v1";
export const LEGACY_CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY = "clovy.home.directTurns.v1";
export const CLOVY_HOME_THREAD_CHANGED_EVENT = "clovy:agent:home-thread-changed";

const homeThreadRetargets = new Map<string, string | null>();

export const CLOVY_HOME_CONTEXT_OPEN = "[Clovy home context]";
export const CLOVY_HOME_CONTEXT_CLOSE = "[/Clovy home context]";
const LEGACY_CLOVY_HOME_CONTEXT_OPEN = "[June home context]";
const LEGACY_CLOVY_HOME_CONTEXT_CLOSE = "[/June home context]";
const CLOVY_HOME_CONTEXT_MARKERS = [
  [CLOVY_HOME_CONTEXT_OPEN, CLOVY_HOME_CONTEXT_CLOSE],
  [LEGACY_CLOVY_HOME_CONTEXT_OPEN, LEGACY_CLOVY_HOME_CONTEXT_CLOSE],
] as const;

export type ClovyHomeTaskRequest = {
  title: string;
  prompt: string;
  summary?: string;
  requiresCurrentResearch?: boolean;
};

export type ClovyHomeCheckIn = {
  createdAt: string;
  text: string;
};

type HomeCheckInRecord = {
  date: string;
  createdAt: string;
};

function storageOrUndefined(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

function readUnknownMap(key: string): Record<string, unknown> {
  try {
    const raw = storageOrUndefined()?.getItem(key);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function readStringMap(key: string): Record<string, string> {
  return Object.fromEntries(
    Object.entries(readUnknownMap(key)).filter((entry): entry is [string, string] => {
      return typeof entry[1] === "string";
    }),
  );
}

function writeJson(key: string, value: unknown): void {
  try {
    storageOrUndefined()?.setItem(key, JSON.stringify(value));
  } catch {
    // Home remains usable for this launch when storage is unavailable.
  }
}

export function readClovyHomeStoredSessionId(profile: string): string | undefined {
  const storedSessionId = readStringMap(CLOVY_HOME_SESSION_IDS_STORAGE_KEY)[profile]?.trim();
  return storedSessionId || undefined;
}

export function writeClovyHomeStoredSessionId(profile: string, storedSessionId: string): void {
  const normalizedProfile = profile.trim() || "default";
  const normalizedSessionId = storedSessionId.trim();
  if (!normalizedSessionId) return;
  // An explicit new profile mapping reactivates this id. This matters when a
  // test fixture or repaired prerelease database deliberately reuses an id
  // that was retired earlier in the same WebView process.
  homeThreadRetargets.delete(normalizedSessionId);
  writeJson(CLOVY_HOME_SESSION_IDS_STORAGE_KEY, {
    ...readStringMap(CLOVY_HOME_SESSION_IDS_STORAGE_KEY),
    [normalizedProfile]: normalizedSessionId,
  });
}

/** Resolve writes owned by an in-flight Home request after its profile was
 * moved or deleted. Moving redirects the late assistant reply into the merged
 * thread; permanent deletion drops it instead of recreating private data. */
export function resolveClovyHomeThreadSessionId(storedSessionId: string): string | undefined {
  let current = storedSessionId;
  const visited = new Set<string>();
  while (!visited.has(current) && homeThreadRetargets.has(current)) {
    visited.add(current);
    const target = homeThreadRetargets.get(current);
    if (!target) return undefined;
    current = target;
  }
  return current;
}

export function retargetClovyHomeThread(sourceSessionId: string, targetSessionId?: string): void {
  const source = sourceSessionId.trim();
  const target = targetSessionId?.trim();
  if (!source || source === target) return;
  homeThreadRetargets.set(source, target || null);
}

export function forgetClovyHomeStoredSessionId(
  profile: string,
  expectedStoredSessionId?: string,
): void {
  const records = readStringMap(CLOVY_HOME_SESSION_IDS_STORAGE_KEY);
  if (expectedStoredSessionId && records[profile] !== expectedStoredSessionId) return;
  if (!(profile in records)) return;
  delete records[profile];
  writeJson(CLOVY_HOME_SESSION_IDS_STORAGE_KEY, records);
}

export function dispatchClovyHomeThreadChanged(storedSessionId: string): void {
  window.dispatchEvent(
    new CustomEvent<{ storedSessionId: string }>(CLOVY_HOME_THREAD_CHANGED_EVENT, {
      detail: { storedSessionId },
    }),
  );
}

function mergedHomeArray(left: unknown, right: unknown): unknown[] {
  const values = [...(Array.isArray(left) ? left : []), ...(Array.isArray(right) ? right : [])];
  const byId = new Map<string, unknown>();
  const withoutId: unknown[] = [];
  for (const value of values) {
    const id =
      value && typeof value === "object" && !Array.isArray(value) && "id" in value
        ? (value as { id?: unknown }).id
        : undefined;
    if (typeof id === "string") byId.set(id, value);
    else withoutId.push(value);
  }
  return [...withoutId, ...byId.values()].sort((leftValue, rightValue) => {
    const createdAt = (value: unknown) =>
      value && typeof value === "object" && !Array.isArray(value) && "createdAt" in value
        ? String((value as { createdAt?: unknown }).createdAt ?? "")
        : "";
    const leftCreatedAt = createdAt(leftValue);
    const rightCreatedAt = createdAt(rightValue);
    return leftCreatedAt && rightCreatedAt ? leftCreatedAt.localeCompare(rightCreatedAt) : 0;
  });
}

function reconcileHomeThreadStore(
  storageKey: string,
  sourceSessionId: string,
  targetSessionId: string | undefined,
): void {
  const records = readUnknownMap(storageKey);
  if (!(sourceSessionId in records)) return;
  if (targetSessionId && targetSessionId !== sourceSessionId) {
    records[targetSessionId] = mergedHomeArray(records[targetSessionId], records[sourceSessionId]);
  }
  if (targetSessionId !== sourceSessionId) delete records[sourceSessionId];
  writeJson(storageKey, records);
}

export type ClovyHomeProfileRemovalPlan = {
  sourceSessionId?: string;
  targetSessionId?: string;
  redundantSessionId?: string;
};

export function clovyHomeProfileRemovalPlan(
  profile: string,
  disposition: "move" | "delete",
): ClovyHomeProfileRemovalPlan {
  const normalizedProfile = profile.trim();
  if (!normalizedProfile || normalizedProfile === "default") return {};
  const sessionIds = readStringMap(CLOVY_HOME_SESSION_IDS_STORAGE_KEY);
  const sourceSessionId = sessionIds[normalizedProfile]?.trim();
  const targetSessionId =
    disposition === "move" ? sessionIds.default?.trim() || sourceSessionId : undefined;
  return {
    ...(sourceSessionId ? { sourceSessionId } : {}),
    ...(targetSessionId ? { targetSessionId } : {}),
    ...(disposition === "move" && sourceSessionId && targetSessionId !== sourceSessionId
      ? { redundantSessionId: sourceSessionId }
      : {}),
  };
}

export function reconcileClovyHomeProfileRemoval(
  profile: string,
  disposition: "move" | "delete",
): ClovyHomeProfileRemovalPlan {
  const normalizedProfile = profile.trim();
  if (!normalizedProfile || normalizedProfile === "default") return {};

  const sessionIds = readStringMap(CLOVY_HOME_SESSION_IDS_STORAGE_KEY);
  const plan = clovyHomeProfileRemovalPlan(profile, disposition);
  const { sourceSessionId, targetSessionId } = plan;

  if (sourceSessionId) {
    retargetClovyHomeThread(sourceSessionId, targetSessionId);
    for (const storageKey of [
      CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY,
      CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY,
      LEGACY_CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY,
      LEGACY_CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY,
    ]) {
      reconcileHomeThreadStore(storageKey, sourceSessionId, targetSessionId);
    }
    if (targetSessionId) dispatchClovyHomeThreadChanged(targetSessionId);
  }

  delete sessionIds[normalizedProfile];
  if (disposition === "move" && targetSessionId) sessionIds.default = targetSessionId;
  writeJson(CLOVY_HOME_SESSION_IDS_STORAGE_KEY, sessionIds);

  const checkIns = readUnknownMap(CLOVY_HOME_CHECK_INS_STORAGE_KEY);
  if (disposition === "move" && !checkIns.default && checkIns[normalizedProfile]) {
    checkIns.default = checkIns[normalizedProfile];
  }
  delete checkIns[normalizedProfile];
  writeJson(CLOVY_HOME_CHECK_INS_STORAGE_KEY, checkIns);

  return plan;
}

export function stripClovyHomeContext(prompt: string): string {
  const trimmed = prompt.trimStart();
  const markers = CLOVY_HOME_CONTEXT_MARKERS.find(([openMarker]) => trimmed.startsWith(openMarker));
  if (!markers) return prompt;
  const closeMarker = markers[1];
  const closeIndex = trimmed.indexOf(closeMarker);
  if (closeIndex < 0) return prompt;
  return trimmed.slice(closeIndex + closeMarker.length).trimStart();
}

export function stripClovyHomeContextFromPreview(preview: string | undefined): string | undefined {
  if (preview === undefined) return undefined;
  const stripped = stripClovyHomeContext(preview);
  if (stripped !== preview) return stripped;
  // A retired runtime may have truncated the preview before the closing marker. Never expose
  // a partial hidden block in lists while the full message remains intact.
  if (
    CLOVY_HOME_CONTEXT_MARKERS.some(([openMarker]) => preview.trimStart().startsWith(openMarker))
  ) {
    return t("lib.home.message");
  }
  return preview;
}

export function isClovyHomeStartTaskTool(name: string | undefined): boolean {
  if (!name) return false;
  const normalized = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_");
  return (
    normalized === "start_task" ||
    normalized.endsWith("clovy_home_start_task") ||
    normalized.endsWith("june_home_start_task")
  );
}

function localDateKey(now: Date): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** Local calendar-day key for a turn timestamp, or "" when unparseable, so the
 * Home transcript can detect day boundaries without inventing one for turns
 * whose timestamps are missing or malformed. */
export function clovyHomeDayKey(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return localDateKey(date);
}

/** Human day-boundary label for the Home thread ("Today at 2:45 PM",
 * "Yesterday at 9:04 AM", "Monday at 8:12 AM", "March 3 at 4:20 PM"). */
export function clovyHomeDayLabel(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const time = formatClockTime(date);
  const startOfDay = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const dayDiff = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
  if (dayDiff <= 0) return t("lib.home.todayAt", { time });
  if (dayDiff === 1) return t("lib.home.yesterdayAt", { time });
  if (dayDiff < 7) return dayAt(formatDate(date, { weekday: "long" }), time);
  const sameYear = date.getFullYear() === now.getFullYear();
  const day = formatDate(
    date,
    sameYear
      ? { month: "long", day: "numeric" }
      : { month: "long", day: "numeric", year: "numeric" },
  );
  return dayAt(day, time);
}

/** "{day} at {time}", capitalized so a lowercase weekday or date
 * ("segunda-feira") still opens the label like "Today" does. */
function dayAt(day: string, time: string): string {
  const label = t("lib.home.dayAt", { day, time });
  return `${label.charAt(0).toLocaleUpperCase(intlLocale())}${label.slice(1)}`;
}

/** The live greeting for the Home surface, derived from the CURRENT clock
 * (unlike the stored check-in, whose text is pinned to its creation time).
 * Early hours read as evening: "Good morning" at 00:37 feels wrong. */
export type ClovyHomeGreetingContext = {
  displayName?: string;
  returning?: boolean;
};

function firstNameFromDisplayName(displayName: string | undefined): string | undefined {
  return displayName?.trim().split(/\s+/)[0] || undefined;
}

export function clovyHomeGreetingParts(
  now = new Date(),
  context: ClovyHomeGreetingContext = {},
): {
  salutation: string;
  question: string;
} {
  const hour = now.getHours();
  const firstName = firstNameFromDisplayName(context.displayName);
  const personalized = (salutation: string) =>
    firstName ? t("lib.home.greetingWithName", { salutation, name: firstName }) : salutation;
  if (hour >= 5 && hour < 12) {
    return {
      salutation: personalized(t("lib.home.goodMorning")),
      question: context.returning ? t("lib.home.pickUpToday") : t("lib.home.helpToday"),
    };
  }
  if (hour >= 12 && hour < 18) {
    return {
      salutation: personalized(t("lib.home.goodAfternoon")),
      question: context.returning ? t("lib.home.pickUpAfternoon") : t("lib.home.helpAfternoon"),
    };
  }
  return {
    salutation: personalized(t("lib.home.goodEvening")),
    question: context.returning ? t("lib.home.pickUpEvening") : t("lib.home.helpEvening"),
  };
}

/** Quiet first-step prompts that follow the user's local day without claiming
 * access to context Clovy has not actually loaded. */
export function clovyHomeNudgePrompts(now = new Date()): readonly string[] {
  const hour = now.getHours();
  if (hour >= 5 && hour < 12) {
    return [
      t("lib.home.nudge.planMyDay"),
      t("lib.home.nudge.thinkThroughDecision"),
      t("lib.home.nudge.getSomethingDone"),
    ];
  }
  if (hour >= 12 && hour < 18) {
    return [
      t("lib.home.nudge.planRestOfDay"),
      t("lib.home.nudge.workThroughBlocker"),
      t("lib.home.nudge.prioritize"),
    ];
  }
  return [
    t("lib.home.nudge.reviewMyDay"),
    t("lib.home.nudge.planTomorrow"),
    t("lib.home.nudge.thinkThroughDecision"),
  ];
}

function checkInText(now: Date): string {
  const greeting = clovyHomeGreetingParts(now);
  return t("lib.home.checkIn", { salutation: greeting.salutation, question: greeting.question });
}

export function clovyHomeDailyCheckIn(profile: string, now = new Date()): ClovyHomeCheckIn {
  let records: Record<string, HomeCheckInRecord> = {};
  try {
    const raw = storageOrUndefined()?.getItem(CLOVY_HOME_CHECK_INS_STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      records = parsed as Record<string, HomeCheckInRecord>;
    }
  } catch {
    records = {};
  }
  const date = localDateKey(now);
  const existing = records[profile];
  const createdAt =
    existing?.date === date && typeof existing.createdAt === "string"
      ? existing.createdAt
      : now.toISOString();
  if (existing?.date !== date || existing.createdAt !== createdAt) {
    writeJson(CLOVY_HOME_CHECK_INS_STORAGE_KEY, {
      ...records,
      [profile]: { date, createdAt },
    });
  }
  return { createdAt, text: checkInText(new Date(createdAt)) };
}
