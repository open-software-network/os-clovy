import { describe, expect, it } from "vitest";
import { modelSpecEntries } from "../components/settings/ModelPickerDialog";
import type { ProviderModelDto } from "../lib/tauri";

function model(overrides: Partial<ProviderModelDto>): ProviderModelDto {
  return {
    provider: "openrouter",
    id: "test-model",
    name: "Test model",
    modelType: "text",
    traits: [],
    capabilities: [],
    ...overrides,
  } as ProviderModelDto;
}

describe("modelSpecEntries", () => {
  it("shows provider USD per million tokens without credits", () => {
    const entries = modelSpecEntries(
      model({
        priceUnit: "tokens",
        pricing: { unit: "per_million_tokens", input: { usd: 2 }, output: { usd: 6 } },
      }),
    );
    expect(entries).toContainEqual({ label: "Input", value: "$2.00 /1M" });
    expect(entries).toContainEqual({ label: "Output", value: "$6.00 /1M" });
  });

  it("falls back to the shared pricing label when there is no credit split", () => {
    const entries = modelSpecEntries(
      model({ priceUnit: "images", priceDescription: "$0.01 per image" }),
    );
    expect(entries).toContainEqual({ label: "Pricing", value: "$0.01 per image" });
  });

  it("appends a formatted context window", () => {
    const entries = modelSpecEntries(model({ contextTokens: 128_000 }));
    expect(entries).toContainEqual({ label: "Context", value: "128K tokens" });
  });
});
