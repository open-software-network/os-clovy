import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { formatTime } from "../components/activity-timeline/timeline-utils";
import { applyInterfaceLocale } from "../i18n/locale";
import { formatNumber, intlLocale, t } from "../i18n/translate";
import { clovyHomeDayLabel } from "../lib/clovy-home";
import { formatCalendarDate, formatListTimestamp } from "../lib/date-format";
import { messageForErrorCode, messageFromError } from "../lib/errors";
import { humanizeSchedule } from "../lib/routine-schedule";

// Every date, time, and named number follows the interface language, never
// the system's (ADR-0061). Each case runs with a system locale that differs
// from the interface language: `__clovyTestSystemLocale` is what a formatter
// given no locale falls back to (see setup.ts), standing in for macOS's
// regional format.
function withSystemLocale(locale: string) {
  globalThis.__clovyTestSystemLocale = locale;
}

/** ICU writes "8:00 AM" with a narrow no-break space. */
function plain(text: string) {
  return text.replace(/[  ]/g, " ");
}

// Thursday, October 15, 2026; three days after Monday, October 12.
const NOW = new Date(2026, 9, 15, 10, 0);
const MONDAY_8AM = new Date(2026, 9, 12, 8, 0).toISOString();

describe("interface language owns every date and time", () => {
  beforeEach(() => applyInterfaceLocale("en"));
  afterEach(() => {
    globalThis.__clovyTestSystemLocale = undefined;
    applyInterfaceLocale("en");
  });

  it("reads Monday at 8:00 AM in English on a Portuguese system", () => {
    withSystemLocale("pt-BR");
    applyInterfaceLocale("en");
    expect(plain(clovyHomeDayLabel(MONDAY_8AM, NOW))).toBe("Monday at 8:00 AM");
    expect(intlLocale()).toBe("en");
    expect(formatListTimestamp(new Date(2026, 9, 12, 8), NOW)).toBe("Mon");
    expect(plain(formatTime(new Date(2026, 9, 15, 9, 30).toISOString()))).toBe("9:30 AM");
    expect(humanizeSchedule("0 8 * * 1").replace(/ /g, " ")).toBe("Every Monday at 8:00 AM");
    expect(formatNumber(1234.5)).toBe("1,234.5");
  });

  it("reads segunda-feira às 08:00 in Portuguese on an English system", () => {
    withSystemLocale("en-US");
    applyInterfaceLocale("pt-BR");
    expect(formatTime(new Date(2026, 9, 15, 9, 30).toISOString())).toBe("09:30");
    expect(clovyHomeDayLabel(MONDAY_8AM, NOW)).toBe("Segunda-feira às 08:00");
    expect(formatListTimestamp(new Date(2026, 9, 12, 8), NOW)).toBe("seg.");
    expect(humanizeSchedule("0 8 * * 1")).toBe("Toda segunda-feira às 08:00");
  });

  it("takes month names from the interface language and day/month order from the preference", () => {
    const date = new Date(2026, 9, 12, 12);
    withSystemLocale("pt-BR");
    applyInterfaceLocale("en");
    // A Portuguese system's order does not apply to an English interface.
    expect(formatCalendarDate(date, "system")).toBe("Oct 12");
    expect(formatCalendarDate(date, "day-first")).toBe("12 Oct");

    // Same language: the system's regional order applies (en-GB is day first).
    withSystemLocale("en-GB");
    expect(formatCalendarDate(date, "system")).toBe("12 Oct");

    withSystemLocale("en-US");
    applyInterfaceLocale("pt-BR");
    // An English system's order does not apply to a Portuguese interface.
    expect(formatCalendarDate(date, "system")).toBe("12 de out.");
    expect(formatCalendarDate(date, "month-first")).toBe("out. 12");

    withSystemLocale("pt-BR");
    expect(formatCalendarDate(date, "system")).toBe("12 de out.");
    expect(formatCalendarDate(date, "day-first")).toBe("12 out.");
  });

  it("writes one clock shape per language across Home, Routines, and the Timeline", () => {
    const at930 = new Date(2026, 9, 15, 9, 30);
    withSystemLocale("en-US");
    applyInterfaceLocale("pt-BR");
    expect(formatTime(at930.toISOString())).toBe("09:30");
    expect(formatListTimestamp(at930, NOW)).toBe("09:30");
    expect(humanizeSchedule("30 9 * * *")).toBe("Todo dia às 09:30");
    expect(clovyHomeDayLabel(at930.toISOString(), NOW)).toBe("Hoje às 09:30");

    withSystemLocale("pt-BR");
    applyInterfaceLocale("en");
    expect(plain(formatTime(at930.toISOString()))).toBe("9:30 AM");
    expect(plain(formatListTimestamp(at930, NOW))).toBe("9:30 AM");
    expect(plain(humanizeSchedule("30 9 * * *"))).toBe("Every day at 9:30 AM");
    expect(plain(clovyHomeDayLabel(at930.toISOString(), NOW))).toBe("Today at 9:30 AM");
  });

  it("phrases stored intervals in the interface language", () => {
    withSystemLocale("en-US");
    applyInterfaceLocale("pt-BR");
    expect(humanizeSchedule("every 30m")).toBe("A cada 30 minutos");
    applyInterfaceLocale("en");
    expect(humanizeSchedule("every 30m")).toBe("Every 30 minutes");
  });
});

describe("native error codes", () => {
  afterEach(() => applyInterfaceLocale("en"));

  it.each([
    ["en", /local model.*Settings > Models/],
    ["pt-BR", /modelo local.*Configurações > Modelos/],
  ] as const)("locates model installation outside Settings in %s", (locale, location) => {
    applyInterfaceLocale(locale);
    expect(
      messageForErrorCode(
        "local_model_not_installed",
        "Install the local model in Settings > Models.",
      ),
    ).toMatch(location);
    expect(
      messageFromError({ code: "local_model_not_installed", message: "native fallback" }),
    ).toMatch(location);
  });

  it("words a known code in the interface language and falls back to the native message", () => {
    applyInterfaceLocale("pt-BR");
    expect(messageForErrorCode("accessibility_permission_missing", "English text")).toBe(
      "O Clovy não conseguiu colar automaticamente. Sua transcrição está na área de transferência, então você pode colar com Cmd+V.",
    );
    expect(
      messageForErrorCode("shortcut_register_failed", "Could not register the shortcut F5.", {
        shortcut: "F5",
      }),
    ).toBe("Não foi possível registrar o atalho F5.");
    expect(messageForErrorCode("some_new_code", "Native text.")).toBe("Native text.");
    expect(messageForErrorCode(undefined, "Native text.")).toBe("Native text.");
    expect(
      messageFromError({
        code: "remote_model_invalid",
        message: "The model selected for this session is invalid. Choose the model again.",
      }),
    ).toBe("O modelo escolhido para esta sessão é inválido. Escolha o modelo de novo.");
  });

  it("keeps English byte-identical to the native wording", () => {
    applyInterfaceLocale("en");
    expect(
      messageForErrorCode(
        "paste_target_unavailable",
        "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
      ),
    ).toBe(
      "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
    );
    expect(messageFromError({ code: "not_signed_in", message: "Sign in to use dictation." })).toBe(
      "Sign in to use dictation.",
    );
  });
});

describe("Direct provider and own-engine errors", () => {
  afterEach(() => applyInterfaceLocale("en"));

  it("localizes direct transport failures without reviving retired service reasons", () => {
    for (const locale of ["en", "pt-BR"] as const) {
      applyInterfaceLocale(locale);
      expect(
        messageForErrorCode(
          "clovy_request_failed",
          "error sending request for url (https://api.groq.com/openai/v1/audio/transcriptions)",
        ),
      ).toBe(t("lib.errors.unreachable"));
      expect(messageForErrorCode("clovy_request_failed", "operation timed out")).toBe(
        t("lib.errors.unreachable"),
      );
      expect(messageForErrorCode("clovy_request_failed", "insufficient_credits")).toBe(
        "insufficient_credits",
      );
      expect(messageForErrorCode("clovy_request_failed", "builder error: invalid URL")).toBe(
        "builder error: invalid URL",
      );
      expect(messageForErrorCode("unrelated", "error sending request")).toBe(
        "error sending request",
      );
    }
  });

  it("localizes direct provider key rejection", () => {
    applyInterfaceLocale("pt-BR");
    expect(
      messageFromError({
        code: "groq_api_key_rejected",
        message: "Groq rejected your saved API key. Open Settings and update it.",
      }),
    ).toBe("Groq recusou sua chave de API salva. Abra os Ajustes e atualize a chave.");
  });

  it("keeps the native detail inside worded own-engine errors", () => {
    applyInterfaceLocale("pt-BR");
    expect(
      messageFromError({ code: "llm_cli_failed", message: "Codex failed: exit status 2" }),
    ).toBe("Codex falhou: exit status 2");
    expect(
      messageFromError({
        code: "llm_endpoint_failed",
        message: "The endpoint returned status 500: upstream exploded",
      }),
    ).toBe("O endpoint respondeu com o status 500: upstream exploded");
    // An unmatched variant of a known code keeps the native message.
    expect(messageFromError({ code: "local_model_invalid", message: "bad url" })).toBe("bad url");
  });

  it("reads exactly like the native English", () => {
    applyInterfaceLocale("en");
    expect(
      messageFromError({
        code: "openrouter_usage_limit_exceeded",
        message: "x",
      }),
    ).toBe(
      "OpenRouter reports that its usage limit was reached. Try again later, or check the plan behind your saved OpenRouter key.",
    );
    expect(
      messageFromError({
        code: "llm_cli_not_installed",
        message: "Pi is not installed on this computer.",
      }),
    ).toBe("Pi is not installed on this computer.");
  });
});
