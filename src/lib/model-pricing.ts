import { formatNumber, isEnglishLocale, t } from "../i18n/translate";
import type { ProviderModelDto } from "./tauri";

export function pricingLabel(model: ProviderModelDto) {
  const tokenPrices = tokenPricingLabels(model);
  if (tokenPrices) return t("lib.modelPricing.perMillionTokens", tokenPrices);
  const pricing = model.pricing;
  if (pricing && typeof pricing === "object") {
    const display = (pricing as Record<string, unknown>).display;
    if (typeof display === "string" && display.trim()) return display.trim();
    const input = priceForPath(pricing, ["input", "usd"]);
    const output = priceForPath(pricing, ["output", "usd"]);
    if (input !== undefined && output !== undefined) {
      return t("lib.modelPricing.inOut", {
        input: usd(formatUsd(input)),
        output: usd(formatUsd(output)),
      });
    }
    const usdValues = collectUsdValues(pricing);
    if (usdValues.length === 1) return usd(formatUsd(usdValues[0]));
    if (usdValues.length > 1) {
      const min = Math.min(...usdValues);
      const max = Math.max(...usdValues);
      return min === max ? usd(formatUsd(min)) : `${usd(formatUsd(min))}-${usd(formatUsd(max))}`;
    }
  }
  if (model.priceDescription?.trim()) return model.priceDescription.trim();
  return undefined;
}

/** Direct provider prices, expressed in USD per million tokens. */
export function tokenPricingLabels(
  model: ProviderModelDto,
): { input: string; output: string } | undefined {
  const pricing = model.pricing;
  if (
    !pricing ||
    typeof pricing !== "object" ||
    !("unit" in pricing) ||
    pricing.unit !== "per_million_tokens"
  )
    return;
  const input = priceForPath(pricing, ["input", "usd"]);
  const output = priceForPath(pricing, ["output", "usd"]);
  if (input === undefined || output === undefined) return;
  return { input: usd(formatUsd(input)), output: usd(formatUsd(output)) };
}

function priceForPath(value: unknown, path: string[]) {
  let current: unknown = value;
  for (const key of path) {
    if (!current || typeof current !== "object" || !(key in current)) {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return typeof current === "number" ? current : undefined;
}

function collectUsdValues(value: unknown): number[] {
  if (!value || typeof value !== "object") return [];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, nested]) => {
    if (key === "usd" && typeof nested === "number") return [nested];
    return collectUsdValues(nested);
  });
}

function formatUsd(value: number) {
  return value >= 1 ? value.toFixed(2) : value.toFixed(4).replace(/0+$/, "0");
}

/** A USD amount given as its English decimal text ("1.75", "0.000150").
 * English keeps the historical `$1.75` shape exactly; other languages format
 * the same amount and precision as US dollars in their own convention
 * ("US$ 1,75"), so the currency never changes, only its notation. */
function usd(amount: string): string {
  if (isEnglishLocale()) return `$${amount}`;
  const digits = amount.split(".")[1]?.length ?? 0;
  return formatNumber(Number(amount), {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}
