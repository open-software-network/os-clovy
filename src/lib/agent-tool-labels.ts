import { t } from "../i18n/translate";

type ToolPayload = Record<string, unknown>;

const GENERIC_TOOL_NAMES = new Set([
  "bash",
  "command",
  "exec",
  "exec_command",
  "run_command",
  "shell",
  "terminal",
]);

export function toolActivityLabel(toolName: string | undefined, payload?: unknown) {
  const records = payloadRecords(payload);
  const rawName =
    nonEmptyString(toolName) ?? firstString(records, ["name", "tool_name", "tool"]) ?? "tool";
  const normalized = normalizeToolName(rawName);
  if (isComputerUseToolName(normalized)) return t("lib.toolActivity.computerUse");
  const command = firstString(records, ["command", "cmd", "script", "shell_command"]);
  const commandLabel = command ? labelFromCommand(command) : undefined;
  if (commandLabel) return commandLabel;

  const payloadLabel = labelFromPayload(records, normalized);
  if (payloadLabel) return payloadLabel;

  const nameLabel = labelFromName(normalized);
  if (nameLabel) return nameLabel;

  return humanizeToolName(rawName);
}

export function toolActivitySentence(toolName: string | undefined, payload?: unknown) {
  const label = toolActivityLabel(toolName, payload);
  return label === t("lib.toolActivity.tool")
    ? t("lib.toolActivity.usingTool")
    : t("lib.toolActivity.sentence", { label });
}

export function humanizeToolName(value: string) {
  const cleaned = value
    .replace(/^tools?[._-]/i, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_./:-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return t("lib.toolActivity.tool");
  const lower = cleaned.toLowerCase();
  return `${lower.charAt(0).toUpperCase()}${lower.slice(1)}`;
}

function labelFromPayload(records: ToolPayload[], normalizedName: string) {
  if (records.some((record) => hasAnyKey(record, ["url", "href", "uri"]))) {
    return t("lib.toolActivity.browsing");
  }
  if (records.some((record) => hasAnyKey(record, ["search_query", "web_query", "url_query"]))) {
    return t("lib.toolActivity.searchingWeb");
  }
  if (records.some((record) => hasAnyKey(record, ["image_query", "image_search_query"]))) {
    return t("lib.toolActivity.searchingImages");
  }
  if (records.some((record) => hasAnyKey(record, ["query", "q"]))) {
    if (isWebToolName(normalizedName)) return t("lib.toolActivity.searchingWeb");
    if (isFileToolName(normalizedName)) return t("lib.toolActivity.searchingFiles");
    return t("lib.toolActivity.searching");
  }
  if (records.some((record) => hasAnyKey(record, ["path", "file", "files"]))) {
    if (isEditToolName(normalizedName)) return t("lib.toolActivity.editingFiles");
    if (isSearchToolName(normalizedName)) return t("lib.toolActivity.searchingFiles");
    return t("lib.toolActivity.readingFiles");
  }
  return undefined;
}

function labelFromCommand(command: string) {
  const lower = command.toLowerCase();
  if (/\bhttps?:\/\//.test(lower) || /\b(curl|wget)\b/.test(lower)) {
    return t("lib.toolActivity.browsing");
  }
  if (/\bgh\s+/.test(lower)) return t("lib.toolActivity.usingGitHub");
  if (/\bgit\s+(status|diff|show|log|grep|ls-files|branch|fetch)\b/.test(lower)) {
    return t("lib.toolActivity.inspectingRepository");
  }
  if (/\b(rg|grep|fd|find)\b/.test(lower)) return t("lib.toolActivity.searchingFiles");
  if (/\b(cat|sed|awk|head|tail|nl|ls|pwd|tree)\b/.test(lower)) {
    return t("lib.toolActivity.readingFiles");
  }
  if (
    /\b(pnpm|npm|yarn|bun)\s+(run\s+)?(test|vitest)\b/.test(lower) ||
    /\b(cargo\s+test|pytest|vitest)\b/.test(lower)
  ) {
    return t("lib.toolActivity.runningTests");
  }
  if (
    /\b(pnpm|npm|yarn|bun)\s+(run\s+)?build\b/.test(lower) ||
    /\b(cargo\s+build|tauri\s+build)\b/.test(lower)
  ) {
    return t("lib.toolActivity.building");
  }
  if (
    /\b(pnpm|npm|yarn|bun)\s+(run\s+)?(lint|check|typecheck)\b/.test(lower) ||
    /\b(eslint|tsc|prettier)\b/.test(lower) ||
    /\bcargo\s+(clippy|fmt|check)\b/.test(lower)
  ) {
    return t("lib.toolActivity.checkingCode");
  }
  if (/\b(git\s+(add|apply|commit|push)|apply_patch)\b/.test(lower)) {
    return t("lib.toolActivity.editingFiles");
  }
  return undefined;
}

function labelFromName(normalizedName: string) {
  if (GENERIC_TOOL_NAMES.has(normalizedName)) return t("lib.toolActivity.runningCommand");
  if (isWebSearchToolName(normalizedName)) return t("lib.toolActivity.searchingWeb");
  if (isWebToolName(normalizedName)) return t("lib.toolActivity.browsing");
  // Video before image: `animate_image` (image-to-video) reads as video work.
  if (isVideoToolName(normalizedName)) return t("lib.toolActivity.workingWithVideo");
  if (isImageToolName(normalizedName)) return t("lib.toolActivity.workingWithImages");
  if (isEditToolName(normalizedName)) return t("lib.toolActivity.editingFiles");
  if (isSearchToolName(normalizedName)) return t("lib.toolActivity.searchingFiles");
  if (isReadToolName(normalizedName)) return t("lib.toolActivity.readingFiles");
  if (hasSegment(normalizedName, ["git", "github", "gh"])) {
    return t("lib.toolActivity.usingGitHub");
  }
  if (hasSegment(normalizedName, ["test", "vitest", "pytest"])) {
    return t("lib.toolActivity.runningTests");
  }
  if (hasSegment(normalizedName, ["build", "compile"])) return t("lib.toolActivity.building");
  if (hasSegment(normalizedName, ["lint", "typecheck", "check"])) {
    return t("lib.toolActivity.checkingCode");
  }
  return undefined;
}

function isWebToolName(value: string) {
  return (
    hasSegment(value, ["browser", "browse", "web", "visit", "navigate"]) ||
    hasPhrase(value, ["fetch_url", "open_url"]) ||
    hasSegment(value, ["http", "url"])
  );
}

function isComputerUseToolName(value: string) {
  return /(^|_)computer_use($|_)/.test(value);
}

function isWebSearchToolName(value: string) {
  return hasPhrase(value, [
    "web_search",
    "search_query",
    "search_web",
    "internet_search",
    "search_internet",
  ]);
}

function isImageToolName(value: string) {
  return hasSegment(value, ["image", "screenshot", "vision"]);
}

function isVideoToolName(value: string) {
  return (
    hasSegment(value, ["video", "animate"]) || hasPhrase(value, ["generate_video", "animate_image"])
  );
}

function isFileToolName(value: string) {
  return isReadToolName(value) || isEditToolName(value) || isSearchToolName(value);
}

function isReadToolName(value: string) {
  return (
    hasSegment(value, ["read", "cat", "list", "ls", "glob", "view"]) ||
    hasPhrase(value, ["open_file", "view_file", "file_read"])
  );
}

function isEditToolName(value: string) {
  return (
    hasSegment(value, ["write", "edit", "patch", "create", "delete", "remove", "move", "copy"]) ||
    hasPhrase(value, ["apply_patch"])
  );
}

function isSearchToolName(value: string) {
  return hasSegment(value, ["search", "grep", "rg", "find", "ripgrep"]);
}

function payloadRecords(payload: unknown): ToolPayload[] {
  const root = objectRecord(payload);
  if (!root) return [];
  const records: ToolPayload[] = [root];
  for (const key of ["arguments", "args", "input", "parameters"]) {
    const child = objectRecord(root[key]);
    if (child) records.push(child);
  }
  return records;
}

function objectRecord(value: unknown): ToolPayload | undefined {
  if (typeof value === "string") {
    try {
      const parsed: unknown = JSON.parse(value);
      return objectRecord(parsed);
    } catch {
      return undefined;
    }
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as ToolPayload;
}

function firstString(records: ToolPayload[], keys: string[]) {
  for (const record of records) {
    for (const key of keys) {
      const value = nonEmptyString(record[key]);
      if (value) return value;
    }
  }
  return undefined;
}

function hasAnyKey(record: ToolPayload, keys: string[]) {
  return keys.some((key) => meaningfulValue(record[key]));
}

function meaningfulValue(value: unknown): boolean {
  if (typeof value === "string") return value.trim().length > 0;
  if (Array.isArray(value)) return value.length > 0;
  if (value && typeof value === "object") return Object.keys(value).length > 0;
  return value !== undefined && value !== null;
}

function nonEmptyString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function normalizeToolName(value: string) {
  return value
    .replace(/^tools?[._-]/i, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[^a-z0-9]+/gi, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
}

function hasSegment(value: string, segments: string[]) {
  const parts = new Set(value.split("_").filter(Boolean));
  return segments.some((segment) => parts.has(segment));
}

function hasPhrase(value: string, phrases: string[]) {
  return phrases.some((phrase) => value.includes(phrase));
}
