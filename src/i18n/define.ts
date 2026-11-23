import type { InterfaceLocale } from "./locale";

/**
 * A plural message picks a form by the `count` parameter using the locale's
 * `Intl.PluralRules` category. `other` is required and is the fallback for any
 * category a catalog does not spell out (for example Portuguese `many`).
 * `zero` is an optional exact-zero override ("No notes yet").
 */
export type PluralMessage = {
  zero?: string;
  one?: string;
  two?: string;
  few?: string;
  many?: string;
  other: string;
};

export type Message = string | PluralMessage;

type MessageShape<M> = M extends string ? string : PluralMessage;

/**
 * Declare one namespace's messages. English is the source of truth: its keys
 * define the namespace, and every other locale must provide the same keys with
 * the same shape (plain string vs plural), so a missing translation is a type
 * error rather than a silent English fallback.
 */
export function defineMessages<const T extends Record<string, Message>>(catalog: {
  en: T;
  "pt-BR": { [K in keyof T]: MessageShape<T[K]> };
}): { [L in InterfaceLocale]: Record<keyof T & string, Message> } {
  return catalog as { [L in InterfaceLocale]: Record<keyof T & string, Message> };
}
