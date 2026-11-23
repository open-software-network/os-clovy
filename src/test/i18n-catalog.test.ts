import { afterEach, describe, expect, it } from "vitest";
import { catalogs, namespaces } from "../i18n/catalog";
import { applyInterfaceLocale } from "../i18n/locale";
import { pricingLabel } from "../lib/model-pricing";
import type { ProviderModelDto } from "../lib/tauri";

afterEach(() => applyInterfaceLocale("en"));

describe("interface catalogs", () => {
  it("keeps namespace keys unique and both languages complete", () => {
    const keys = Object.values(namespaces).flatMap((namespace) => Object.keys(namespace.en));
    expect(new Set(keys).size).toBe(keys.length);
    expect(Object.keys(catalogs["pt-BR"]).sort()).toEqual(Object.keys(catalogs.en).sort());
  });

  it.each([
    ["en", "$2.00 input / $6.00 output per 1M tokens"],
    ["pt-BR", "US$ 2,00 entrada / US$ 6,00 saída por 1M de tokens"],
  ] as const)("formats direct provider token prices in %s without credits", (locale, expected) => {
    applyInterfaceLocale(locale);
    const model: ProviderModelDto = {
      id: "vendor/text",
      name: "Text",
      provider: "openrouter",
      modelType: "text",
      capabilities: [],
      traits: [],
      priceUnit: "tokens",
      pricing: { unit: "per_million_tokens", input: { usd: 2 }, output: { usd: 6 } },
    };
    expect(pricingLabel(model)?.replace(/\u00a0/g, " ")).toBe(expected);
  });
});
