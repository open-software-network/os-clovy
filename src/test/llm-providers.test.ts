import { describe, expect, it } from "vitest";
import {
  isActivityEligible,
  parseProviderRefKey,
  providerRefKey,
  type LlmProviderRef,
} from "../lib/llm-providers";

describe("llm-providers pure helpers", () => {
  it("determines activity eligibility correctly", () => {
    expect(isActivityEligible("strict")).toBe(true);
    expect(isActivityEligible("json_schema")).toBe(true);
    expect(isActivityEligible("json_object")).toBe(false);
    expect(isActivityEligible("prompt")).toBe(false);
    expect(isActivityEligible("none")).toBe(false);
    expect(isActivityEligible(undefined)).toBe(false);
  });

  it("encodes and decodes providerRefKey correctly", () => {
    const openrouter: LlmProviderRef = { kind: "openrouter" };
    const none: LlmProviderRef = { kind: "none" };
    const endpoint: LlmProviderRef = { kind: "endpoint", id: "ep-123" };
    const cli: LlmProviderRef = { kind: "cli", id: "codex" };

    expect(providerRefKey(openrouter)).toBe("openrouter");
    expect(providerRefKey(none)).toBe("none");
    expect(providerRefKey(endpoint)).toBe("endpoint:ep-123");
    expect(providerRefKey(cli)).toBe("cli:codex");

    expect(parseProviderRefKey("openrouter")).toEqual(openrouter);
    expect(parseProviderRefKey("none")).toEqual(none);
    expect(parseProviderRefKey("endpoint:ep-123")).toEqual(endpoint);
    expect(parseProviderRefKey("cli:codex")).toEqual(cli);
    expect(parseProviderRefKey("invalid")).toEqual({ kind: "none" });
  });
});
