import { type MessageKey, t } from "../i18n/translate";

/**
 * Languages the transcription providers accept as a hint. Shared by the
 * settings page and the onboarding wizard so the two pickers can't drift.
 *
 * `value` is the transcription language code sent to the provider and never
 * changes; `label` is the language's name in the interface language (this is
 * the dictation language, not the interface language).
 */
function option(value: string, key: MessageKey): { value: string; label: string } {
  return {
    value,
    get label() {
      return t(key);
    },
  };
}

export const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  option("", "lib.dictationLanguage.auto"),
  option("en", "lib.dictationLanguage.en"),
  option("ar", "lib.dictationLanguage.ar"),
  option("zh", "lib.dictationLanguage.zh"),
  option("nl", "lib.dictationLanguage.nl"),
  option("fr", "lib.dictationLanguage.fr"),
  option("de", "lib.dictationLanguage.de"),
  option("hi", "lib.dictationLanguage.hi"),
  option("id", "lib.dictationLanguage.id"),
  option("it", "lib.dictationLanguage.it"),
  option("ja", "lib.dictationLanguage.ja"),
  option("ko", "lib.dictationLanguage.ko"),
  option("no", "lib.dictationLanguage.no"),
  option("pl", "lib.dictationLanguage.pl"),
  option("pt", "lib.dictationLanguage.pt"),
  option("ru", "lib.dictationLanguage.ru"),
  option("es", "lib.dictationLanguage.es"),
  option("sv", "lib.dictationLanguage.sv"),
  option("th", "lib.dictationLanguage.th"),
  option("tr", "lib.dictationLanguage.tr"),
  option("uk", "lib.dictationLanguage.uk"),
  option("vi", "lib.dictationLanguage.vi"),
];

export function languageLabel(value: string) {
  return LANGUAGE_OPTIONS.find((option) => option.value === value)?.label ?? value;
}
