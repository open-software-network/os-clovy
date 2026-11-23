import { clockTimeOptions, formatNumber, intlLocale, isEnglishLocale } from "../../i18n";

export function formatBytes(value: number | null | undefined) {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const amount =
    size >= 10 || unit === 0
      ? Math.round(size)
      : isEnglishLocale()
        ? size.toFixed(1)
        : formatNumber(size, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  return `${amount} ${units[unit]}`;
}

export function relativeDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(intlLocale(), {
    month: "short",
    day: "numeric",
    ...clockTimeOptions(),
  }).format(date);
}
