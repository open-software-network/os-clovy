import { Fragment, type ReactNode, createElement, useCallback, useSyncExternalStore } from "react";
import {
  DEFAULT_LOCALE,
  getInterfaceLocale,
  type InterfaceLocale,
  subscribeInterfaceLocale,
} from "./locale";
import {
  formatParamNumber,
  interpolate,
  type MessageKey,
  resolveTemplate,
  t,
  type TranslateParams,
} from "./translate";

function subscribe(onChange: () => void) {
  return subscribeInterfaceLocale(onChange);
}

/** The current interface locale; re-renders the caller when it changes. */
export function useLocale(): InterfaceLocale {
  return useSyncExternalStore(subscribe, getInterfaceLocale, () => DEFAULT_LOCALE);
}

export type TFunction = (key: MessageKey, params?: TranslateParams) => string;

/**
 * Translator bound to the current interface locale. Every component that
 * renders translated text calls this hook (rather than the bare `t`) so it
 * re-renders when the language changes, including memoized children.
 */
export function useT(): TFunction {
  const locale = useLocale();
  return useCallback((key, params) => t(key, params, locale), [locale]);
}

export type RichParams = Record<
  string,
  string | number | ReactNode | ((chunks: ReactNode) => ReactNode) | undefined | null
>;

const RICH_TOKEN = /<(\w+)>(.*?)<\/\1>|\{(\w+)\}/g;

/**
 * Translate a message that embeds markup: `<tag>text</tag>` spans are rendered
 * by a `tag: (chunks) => node` param, and `{name}` placeholders accept React
 * nodes as well as strings and numbers. Keeps word order in the catalog so a
 * translation can move the link or the bold span freely.
 */
export function translateRich(
  key: MessageKey,
  params: RichParams = {},
  locale: InterfaceLocale = getInterfaceLocale(),
): ReactNode {
  const plainParams: TranslateParams = {};
  for (const [name, value] of Object.entries(params)) {
    if (typeof value === "string" || typeof value === "number") plainParams[name] = value;
  }
  const template = resolveTemplate(key, plainParams, locale);
  const nodes: ReactNode[] = [];
  let cursor = 0;
  let index = 0;
  for (const match of template.matchAll(RICH_TOKEN)) {
    const start = match.index ?? 0;
    if (start > cursor) nodes.push(template.slice(cursor, start));
    const [whole, tag, inner, name] = match;
    if (tag) {
      const render = params[tag];
      const chunks = interpolate(inner, plainParams, locale);
      nodes.push(
        createElement(
          Fragment,
          { key: index },
          typeof render === "function" ? render(chunks) : chunks,
        ),
      );
    } else if (name) {
      const value = params[name];
      let node: ReactNode = whole;
      if (typeof value === "number") node = formatParamNumber(value, locale);
      else if (value !== undefined && value !== null && typeof value !== "function") node = value;
      nodes.push(createElement(Fragment, { key: index }, node));
    }
    cursor = start + whole.length;
    index += 1;
  }
  if (cursor < template.length) nodes.push(template.slice(cursor));
  return createElement(Fragment, null, ...nodes);
}

export function useTRich() {
  const locale = useLocale();
  return useCallback(
    (key: MessageKey, params?: RichParams) => translateRich(key, params, locale),
    [locale],
  );
}
