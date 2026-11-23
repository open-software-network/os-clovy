import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { applyInterfaceLocale } from "../i18n/locale";
import { toolActivityLabel, toolActivitySentence } from "../lib/agent-tool-labels";
import { autonomyUnlockHint } from "../lib/connectors";
import { LANGUAGE_OPTIONS, languageLabel } from "../lib/dictation-languages";
import { FONT_SCALE_PRESETS } from "../lib/font-scale";
import { compactScheduleLabel, humanizeSchedule } from "../lib/routine-schedule";
import type { ConnectorPolicyCatalog } from "../lib/tauri";

const EXPECTED_LANGUAGE_VALUES = [
  "",
  "en",
  "ar",
  "zh",
  "nl",
  "fr",
  "de",
  "hi",
  "id",
  "it",
  "ja",
  "ko",
  "no",
  "pl",
  "pt",
  "ru",
  "es",
  "sv",
  "th",
  "tr",
  "uk",
  "vi",
];

const policy = { earnedAutonomyMinApprovalRuns: 3 } as ConnectorPolicyCatalog;

describe("lib copy in English (default)", () => {
  it("keeps the original English labels", () => {
    expect(languageLabel("pt")).toBe("Portuguese");
    expect(LANGUAGE_OPTIONS[0].label).toBe("Auto-detect");
    expect(FONT_SCALE_PRESETS.map((preset) => preset.label)).toEqual([
      "Default",
      "Large",
      "Larger",
    ]);
    expect(toolActivityLabel("web_search")).toBe("Searching web");
    expect(humanizeSchedule("*/15 * * * *")).toBe("Every 15 minutes");
    expect(autonomyUnlockHint(policy, 2)).toBe(
      "Runs 1 more time under approval to unlock autonomous.",
    );
  });
});

describe("lib copy in Brazilian Portuguese", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));
  afterEach(() => applyInterfaceLocale("en"));

  it("translates dictation language names while the language codes stay unchanged", () => {
    expect(LANGUAGE_OPTIONS.map((option) => option.value)).toEqual(EXPECTED_LANGUAGE_VALUES);
    expect(LANGUAGE_OPTIONS[0].label).toBe("Detectar automaticamente");
    expect(languageLabel("pt")).toBe("Português");
    expect(languageLabel("de")).toBe("Alemão");
    expect(languageLabel("en")).toBe("Inglês");
    // Unknown codes still echo the raw value.
    expect(languageLabel("xx")).toBe("xx");
  });

  it("follows a language switch without re-importing", () => {
    applyInterfaceLocale("en");
    expect(languageLabel("es")).toBe("Spanish");
    applyInterfaceLocale("pt-BR");
    expect(languageLabel("es")).toBe("Espanhol");
    expect(FONT_SCALE_PRESETS.map((preset) => preset.label)).toEqual(["Padrão", "Grande", "Maior"]);
  });

  it("translates tool activity labels", () => {
    expect(toolActivityLabel("web_search")).toBe("Buscando na web");
    expect(toolActivityLabel("bash", { command: "pnpm test" })).toBe("Executando testes");
    expect(toolActivitySentence("read_file", { path: "a.ts" })).toBe("Lendo arquivos.");
    expect(toolActivitySentence("___")).toBe("Usando uma ferramenta.");
  });

  it("picks plural forms for counted copy", () => {
    expect(humanizeSchedule("*/15 * * * *")).toBe("A cada 15 minutos");
    expect(humanizeSchedule("0 */2 * * *")).toBe("A cada 2 horas");
    expect(autonomyUnlockHint(policy, 2)).toBe(
      "Execute mais 1 vez com aprovação para liberar o modo autônomo.",
    );
    expect(autonomyUnlockHint(policy, 0)).toBe(
      "Execute mais 3 vezes com aprovação para liberar o modo autônomo.",
    );
  });

  it("describes routine schedules with Portuguese weekday and time names", () => {
    expect(humanizeSchedule("0 9 * * 1-5")).toBe("Todo dia útil às 09:00");
    expect(humanizeSchedule("0 8 * * *")).toBe("Todo dia às 08:00");
    expect(humanizeSchedule("30 17 * * 1")).toBe("Toda segunda-feira às 17:30");
    expect(humanizeSchedule("0 10 * * 6")).toBe("Todo sábado às 10:00");
    expect(humanizeSchedule("0 8 * * 1-4")).toBe("De segunda-feira a quinta-feira às 08:00");
    expect(humanizeSchedule("0 9 1,15 * *")).toBe("Todo mês nos dias 1 e 15 às 09:00");
    expect(compactScheduleLabel("0 9 * * 1-5")).toBe("Todo dia útil 09:00");
    // The stored interval grammar stays English on disk but reads in Portuguese.
    expect(humanizeSchedule("every 30m")).toBe("A cada 30 minutos");
    expect(humanizeSchedule("every 2h")).toBe("A cada 2 horas");
    expect(compactScheduleLabel("every 30m")).toBe("A cada 30 minutos");
    // Free text is still returned verbatim.
    expect(humanizeSchedule("every day at 9:00")).toBe("every day at 9:00");
  });
});
