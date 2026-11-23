import { describe, expect, it } from "vitest";
import {
  ANONYMOUS_MODEL_DESCRIPTION,
  E2EE_MODEL_DESCRIPTION,
  PRIVATE_MODEL_DESCRIPTION,
  modelIsPrivate,
  modelAvailableForMode,
  modelPrivacyBadge,
  modelSupportsImageInput,
} from "../lib/model-privacy";
import type { ProviderModelDto } from "../lib/tauri";

// Excess-property checks would reject a `traits` field on the narrowed
// capabilities-only param, so route test shapes through a Partial helper.
const model = (partial: Partial<ProviderModelDto>): Partial<ProviderModelDto> => partial;

describe("model privacy labels", () => {
  it("uses e2ee mode over private — the stronger claim wins", () => {
    expect(
      modelPrivacyBadge({
        privacy: "private",
        traits: [],
        capabilities: ["e2ee"],
      }),
    ).toMatchObject({
      mode: "e2ee",
      label: "E2EE",
      description: E2EE_MODEL_DESCRIPTION,
    });
  });

  it("reads the e2ee signal from privacy, traits, or capabilities", () => {
    expect(modelPrivacyBadge({ privacy: "e2ee", traits: [] })?.mode).toBe("e2ee");
    expect(modelPrivacyBadge({ privacy: "", traits: ["e2ee"] })?.mode).toBe("e2ee");
    expect(modelPrivacyBadge({ privacy: "", traits: [], capabilities: ["E2EE"] })?.mode).toBe(
      "e2ee",
    );
  });

  it("uses private mode for private models even when they are anonymized", () => {
    expect(modelPrivacyBadge({ privacy: "private", traits: ["anonymized"] })).toMatchObject({
      mode: "private",
      label: "Private mode",
      description: PRIVATE_MODEL_DESCRIPTION,
    });
  });

  it("uses anonymous mode for anonymous-only models", () => {
    expect(modelPrivacyBadge({ privacy: "anonymous", traits: [] })).toMatchObject({
      mode: "anonymous",
      label: "Anonymous mode",
      description: ANONYMOUS_MODEL_DESCRIPTION,
    });
  });

  it("does not label models without a privacy signal", () => {
    expect(modelPrivacyBadge({ privacy: "OpenAI", traits: ["prompt"] })).toBe(undefined);
  });
});

describe("private catalog filter", () => {
  it("keeps zero-retention-or-stronger models: private, e2ee, and loopback local", () => {
    expect(modelIsPrivate({ privacy: "private", traits: [] })).toBe(true);
    expect(modelIsPrivate({ privacy: "", traits: ["e2ee"] })).toBe(true);
    // A loopback local model never leaves the machine — stronger than any
    // provider retention claim, so the filter must not hide it.
    expect(modelIsPrivate({ privacy: "local", traits: [] })).toBe(true);
  });

  it("drops anonymized models and custom endpoints that make no claim", () => {
    expect(modelIsPrivate({ privacy: "anonymized", traits: [] })).toBe(false);
    // A non-loopback custom endpoint reports "external" — no privacy claim.
    expect(modelIsPrivate({ privacy: "external", traits: [] })).toBe(false);
    expect(modelIsPrivate({ privacy: "", traits: [] })).toBe(false);
  });
});

describe("model mode availability", () => {
  it("excludes the retired Auto router", () => {
    expect(
      modelAvailableForMode("generation", {
        id: "open-software/auto",
        provider: "june",
        capabilities: [],
      }),
    ).toBe(false);
  });
});

describe("model image input support", () => {
  it("is true when the authoritative supportsVision capability is present", () => {
    expect(modelSupportsImageInput({ capabilities: ["supportsVision"] })).toBe(true);
  });

  it("recognizes a real vision model (Fable/Kimi shape: vision + tools)", () => {
    expect(
      modelSupportsImageInput({
        capabilities: ["supportsFunctionCalling", "supportsVision", "supportsMultipleImages"],
      }),
    ).toBe(true);
  });

  it("ignores descriptive traits — a 'multimodal' trait is not vision (JUN-165)", () => {
    // Marketing/descriptive traits conflate image OUTPUT with image INPUT, so
    // they must never make a non-vision model look vision-capable, or the
    // image-attach fallback would switch to a model that can't read the image.
    expect(
      modelSupportsImageInput(
        model({
          capabilities: ["supportsFunctionCalling"],
          traits: ["multimodal"],
        }),
      ),
    ).toBe(false);
    expect(
      modelSupportsImageInput(model({ capabilities: [], traits: ["multimodal", "uncensored"] })),
    ).toBe(false);
  });

  it("is false for a non-vision model (GLM 5.2 shape: tools, no vision)", () => {
    expect(
      modelSupportsImageInput({
        capabilities: ["supportsFunctionCalling", "supportsReasoning", "supportsWebSearch"],
      }),
    ).toBe(false);
  });

  it("is false when no capabilities are reported", () => {
    expect(modelSupportsImageInput({})).toBe(false);
    expect(modelSupportsImageInput({ capabilities: [] })).toBe(false);
  });
});
