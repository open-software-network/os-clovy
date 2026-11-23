import { t } from "../i18n/translate";
import type { AgentSkillDocument, AgentSkillInfo } from "./tauri";

const EXPLICIT_SKILLS_START = "---EXPLICIT SKILLS---";
const EXPLICIT_SKILLS_END = "---END EXPLICIT SKILLS---";
const USER_REQUEST_START = "---USER REQUEST---";
const USER_REQUEST_END = "---END USER REQUEST---";

export type ParsedSkillSlashCommands = {
  commandNames: string[];
  prompt: string;
};

export type ParsedSkillSlashCommandToken = {
  name: string;
  from: number;
  to: number;
};

export type SkillSlashResolution =
  | { status: "resolved"; token: string; skill: AgentSkillInfo }
  | {
      status: "missing";
      token: string;
      suggestions: AgentSkillInfo[];
    }
  | {
      status: "ambiguous";
      token: string;
      matches: AgentSkillInfo[];
    }
  | {
      status: "disabled";
      token: string;
      matches: AgentSkillInfo[];
    };

export function parseSkillSlashCommands(input: string): ParsedSkillSlashCommands {
  const commands = parseSkillSlashCommandTokens(input);
  const commandNames: string[] = [];
  const seen = new Set<string>();
  for (const command of commands) {
    const key = normalizeSkillName(command.name);
    if (seen.has(key)) continue;
    seen.add(key);
    commandNames.push(command.name);
  }
  return {
    commandNames,
    prompt: input.slice(commands.at(-1)?.to ?? 0).trimStart(),
  };
}

export function parseSkillSlashCommandTokens(input: string): ParsedSkillSlashCommandToken[] {
  let index = 0;
  const commands: ParsedSkillSlashCommandToken[] = [];

  while (index < input.length) {
    while (index < input.length && /\s/.test(input[index])) index += 1;
    if (input[index] !== "/") break;

    const from = index;
    const commandStart = index + 1;
    index = commandStart;
    while (index < input.length && !/\s/.test(input[index])) index += 1;

    const command = input.slice(commandStart, index).trim();
    if (!command) break;

    commands.push({ name: command, from, to: index });
  }

  return commands;
}

export function matchSkillSlashSuggestions(
  query: string,
  skills: AgentSkillInfo[] | null | undefined,
  limit = 8,
) {
  const normalized = normalizeSkillName(query);
  return (skills ?? [])
    .filter(isSkillEnabled)
    .map((skill) => ({ skill, score: skillMatchScore(skill, normalized) }))
    .filter((item) => item.score > 0)
    .sort(
      (a, b) => b.score - a.score || safeText(a.skill.name).localeCompare(safeText(b.skill.name)),
    )
    .slice(0, limit)
    .map((item) => item.skill);
}

export function resolveSkillSlashCommands(
  commandNames: string[],
  skills: AgentSkillInfo[],
): SkillSlashResolution[] {
  const enabledSkills = skills.filter(isSkillEnabled);
  const disabledSkills = skills.filter((skill) => !isSkillEnabled(skill));
  return commandNames.map((token) => {
    const matches = matchingSkills(token, enabledSkills);
    if (matches.length === 1) {
      return { status: "resolved", token, skill: matches[0] };
    }
    if (matches.length > 1) {
      return { status: "ambiguous", token, matches };
    }
    const disabledMatches = matchingSkills(token, disabledSkills);
    if (disabledMatches.length) {
      return { status: "disabled", token, matches: disabledMatches };
    }
    return {
      status: "missing",
      token,
      suggestions: matchSkillSlashSuggestions(token, enabledSkills, 3),
    };
  });
}

export function skillSlashResolutionError(resolution: SkillSlashResolution) {
  if (resolution.status === "resolved") return null;
  if (resolution.status === "ambiguous") {
    const matches = resolution.matches
      .slice(0, 4)
      .map((skill) => `/${skill.name}`)
      .join(", ");
    return t("lib.skillSlash.ambiguous", { token: resolution.token, matches });
  }
  if (resolution.status === "disabled") {
    const matches = resolution.matches
      .slice(0, 4)
      .map((skill) => `/${skill.name}`)
      .join(", ");
    if (!matches) {
      return t("lib.skillSlash.disabledToken", { token: resolution.token });
    }
    return resolution.matches.length === 1
      ? t("lib.skillSlash.disabledOne", { matches })
      : t("lib.skillSlash.disabledMany", { matches });
  }
  const suggestions = resolution.suggestions.map((skill) => `/${skill.name}`).join(", ");
  return suggestions
    ? t("lib.skillSlash.missingWithSuggestions", { token: resolution.token, suggestions })
    : t("lib.skillSlash.missing", { token: resolution.token });
}

export function explicitSkillInvocationPrompt(documents: AgentSkillDocument[], request: string) {
  const skillBlocks = documents.map((document) =>
    [
      `Skill: ${document.name}`,
      document.relativePath ? `Path: ${document.relativePath}` : null,
      "",
      document.content.trim(),
    ]
      .filter((line): line is string => line !== null)
      .join("\n"),
  );
  return [
    EXPLICIT_SKILLS_START,
    "The user explicitly invoked these skills for this turn. Follow each selected skill's instructions before handling the user request. Use normal automatic skill matching for any additional relevant skills.",
    "",
    ...skillBlocks.flatMap((block, index) => (index === 0 ? [block] : ["", block])),
    EXPLICIT_SKILLS_END,
    "",
    USER_REQUEST_START,
    request,
    USER_REQUEST_END,
  ].join("\n");
}

export function displayedSkillInvocationText(content: string): string {
  const text = content.trim();
  if (!text.startsWith(EXPLICIT_SKILLS_START)) return content;

  const skillsEnd = text.indexOf(EXPLICIT_SKILLS_END);
  if (skillsEnd === -1) return content;

  const start = text.indexOf(USER_REQUEST_START, skillsEnd + EXPLICIT_SKILLS_END.length);
  if (start === -1) return content;
  const end = text.lastIndexOf(USER_REQUEST_END);
  if (end <= start) return content;
  if (text.slice(end + USER_REQUEST_END.length).trim()) return content;

  const request = text.slice(start + USER_REQUEST_START.length, end).trim();
  return request || content;
}

export function skillDocumentLookupName(name: string) {
  const parts = name.trim().split(/[/:]/).filter(Boolean);
  return parts.at(-1) ?? name.trim();
}

export function isPathLikeSlashToken(token: string) {
  return token.includes("/") || token.includes("\\");
}

function matchingSkills(token: string, skills: AgentSkillInfo[]) {
  const normalized = normalizeSkillName(token);
  const exact = skills.filter((skill) => normalizeSkillName(skill.name) === normalized);
  if (exact.length) return exact;
  return skills.filter((skill) => skillAliases(skill.name).some((alias) => alias === normalized));
}

function skillMatchScore(skill: AgentSkillInfo, query: string) {
  if (!query) return 1;
  const name = normalizeSkillName(skill.name);
  if (name === query) return 100;
  if (skillAliases(skill.name).some((alias) => alias === query)) return 90;
  if (name.startsWith(query)) return 80;
  if (skillAliases(skill.name).some((alias) => alias.startsWith(query))) {
    return 70;
  }
  if (name.includes(query)) return 60;
  const description = normalizeSkillName(skill.description ?? "");
  const category = normalizeSkillName(skill.category ?? "");
  if (description.includes(query) || category.includes(query)) return 40;
  return 0;
}

function skillAliases(name: string) {
  return name.split(/[/:]/).map(normalizeSkillName).filter(Boolean);
}

function normalizeSkillName(value: string) {
  return value.trim().toLowerCase();
}

function safeText(value: unknown) {
  return typeof value === "string" ? value : "";
}

function isSkillEnabled(skill: AgentSkillInfo) {
  return skill.enabled !== false;
}
