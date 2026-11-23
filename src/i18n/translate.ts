import { catalogs, type MessageKey } from "./catalog";
import type { Message, PluralMessage } from "./define";
import { DEFAULT_LOCALE, getInterfaceLocale, type InterfaceLocale } from "./locale";

export type { MessageKey };
export type TranslateParams = Record<string, string | number | undefined | null>;

const pluralRulesCache = new Map<InterfaceLocale, Intl.PluralRules>();
const numberFormatCache = new Map<string, Intl.NumberFormat>();

function pluralRules(locale: InterfaceLocale) {
  let rules = pluralRulesCache.get(locale);
  if (!rules) {
    rules = new Intl.PluralRules(locale);
    pluralRulesCache.set(locale, rules);
  }
  return rules;
}

/**
 * The BCP 47 tag handed to `Intl` formatters: always the interface language,
 * never the system's. Weekday and month names, AM/PM, and separators must
 * match the words around them, so an English UI on a Portuguese system reads
 * "Monday at 8:00 AM" and a Portuguese UI on an English system reads
 * "segunda-feira às 08:00" (ADR-0061).
 */
export function intlLocale(locale: InterfaceLocale = getInterfaceLocale()): string {
  return locale;
}

/** Whether `locale` is English, where copy keeps the historical fixed shapes
 * (raw digits, `toFixed` decimals, `$1.75`). */
export function isEnglishLocale(locale: InterfaceLocale = getInterfaceLocale()): boolean {
  return locale === "en";
}

export function formatNumber(
  value: number,
  options?: Intl.NumberFormatOptions,
  locale: InterfaceLocale = getInterfaceLocale(),
) {
  const cacheKey = `${locale}|${options ? JSON.stringify(options) : ""}`;
  let formatter = numberFormatCache.get(cacheKey);
  if (!formatter) {
    formatter = new Intl.NumberFormat(intlLocale(locale), options);
    numberFormatCache.set(cacheKey, formatter);
  }
  return formatter.format(value);
}

export function formatDate(
  value: Date | number | string,
  options?: Intl.DateTimeFormatOptions,
  locale: InterfaceLocale = getInterfaceLocale(),
) {
  const date = value instanceof Date ? value : new Date(value);
  return new Intl.DateTimeFormat(intlLocale(locale), options).format(date);
}

/**
 * The one clock-time shape for every visible time: "8:00 AM" in English and
 * zero-padded 24-hour "08:00" in Portuguese. Spread it into wider options
 * (`{ month: "short", day: "numeric", ...clockTimeOptions() }`).
 */
export function clockTimeOptions(
  locale: InterfaceLocale = getInterfaceLocale(),
): Intl.DateTimeFormatOptions {
  return { hour: isEnglishLocale(locale) ? "numeric" : "2-digit", minute: "2-digit" };
}

/** A clock time ("8:00 AM", "08:00") in the interface language. */
export function formatClockTime(
  value: Date | number | string,
  locale: InterfaceLocale = getInterfaceLocale(),
) {
  return formatDate(value, clockTimeOptions(locale), locale);
}

export function formatList(
  items: string[],
  options?: Intl.ListFormatOptions,
  locale: InterfaceLocale = getInterfaceLocale(),
) {
  return new Intl.ListFormat(intlLocale(locale), options).format(items);
}

function lookup(locale: InterfaceLocale, key: string): Message | undefined {
  return catalogs[locale][key] ?? catalogs[DEFAULT_LOCALE][key];
}

function selectPlural(message: PluralMessage, count: number, locale: InterfaceLocale): string {
  if (count === 0 && message.zero !== undefined) return message.zero;
  // CLDR files Portuguese 0 under `one` ("0 nota"); Brazilian usage reads
  // zero as plural ("0 notas"), so zero takes the `other` form.
  const category =
    locale === "pt-BR" && count === 0 ? "other" : pluralRules(locale).select(Math.abs(count));
  return message[category as keyof PluralMessage] ?? message.other;
}

/**
 * A number placed inside translated copy. English keeps the raw digits the
 * copy always used (`${count}`: "1234 notes", never regrouped by the system
 * locale); other languages format with their own separators ("1.234 notas").
 */
export function formatParamNumber(value: number, locale: InterfaceLocale) {
  return isEnglishLocale(locale) ? String(value) : formatNumber(value, undefined, locale);
}

const PLACEHOLDER = /\{(\w+)\}/g;

export function interpolate(
  template: string,
  params: TranslateParams | undefined,
  locale: InterfaceLocale,
) {
  if (!params) return template;
  return template.replace(PLACEHOLDER, (match, name: string) => {
    const value = params[name];
    if (value === undefined || value === null) return match;
    return typeof value === "number" ? formatParamNumber(value, locale) : value;
  });
}

/** Resolve the raw template for a key (plural form already selected). */
export function resolveTemplate(
  key: MessageKey,
  params: TranslateParams | undefined,
  locale: InterfaceLocale,
): string {
  const message = lookup(locale, key);
  if (message === undefined) return key;
  if (typeof message === "string") return message;
  const count = Number(params?.count ?? 0);
  return selectPlural(message, Number.isFinite(count) ? count : 0, locale);
}

/**
 * Translate a key into the current interface language. Unknown keys return
 * the key itself (visible in review, never a crash); a key missing from a
 * non-English catalog falls back to English.
 *
 * Numeric params stay raw in English and are formatted with the locale's
 * digits and separators elsewhere; pass a string (`String(year)`) for numbers
 * that must stay raw in every language.
 */
export function t(
  key: MessageKey,
  params?: TranslateParams,
  locale: InterfaceLocale = getInterfaceLocale(),
): string {
  return interpolate(resolveTemplate(key, params, locale), params, locale);
}
