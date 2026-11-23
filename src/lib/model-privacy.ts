import { type MessageKey, t } from "../i18n/translate";
const AUTO_MODEL_ID = "open-software/auto";
import type { BuiltinProvider, ProviderModelDto, ProviderModelMode } from "./tauri";

export type ModelPrivacyMode = "e2ee" | "private" | "anonymous";

export type ModelPrivacyBadge = {
  mode: ModelPrivacyMode;
  label: string;
  description: string;
};

export type ModelPrivacyFlags = {
  e2ee: boolean;
  private: boolean;
  anonymous: boolean;
  uncensored: boolean;
};

export const PROVIDER_MODEL_SETTINGS_CHANGED_EVENT = "clovy:provider-model-settings-changed";

export type ProviderModelSettingsChangedDetail = {
  mode: ProviderModelMode;
  modelId: string;
};

export function dispatchProviderModelSettingsChanged(detail: ProviderModelSettingsChangedDetail) {
  window.dispatchEvent(
    new CustomEvent<ProviderModelSettingsChangedDetail>(PROVIDER_MODEL_SETTINGS_CHANGED_EVENT, {
      detail,
    }),
  );
}

// English reference copy (the badge itself reads the interface language from
// the `lib.modelPrivacy.*` catalog keys at call time).
export const E2EE_MODEL_DESCRIPTION =
  "Private model with end-to-end encryption. Your prompt is encrypted on your device and only decrypted inside a hardware-secured enclave (TEE); the response is encrypted before it leaves the enclave. No prompt data is ever readable by the model provider or its infrastructure.";
export const PRIVATE_MODEL_DESCRIPTION =
  "Private model with zero data retention. No prompt data is stored, shared with a third party, or trained on.";
export const ANONYMOUS_MODEL_DESCRIPTION =
  "The model provider may retain prompts, though they're anonymized. Your identity is stripped before anything leaves Clovy. For sensitive content, pick a Private or E2EE model.";

type ModelPrivacySignals = Pick<ProviderModelDto, "privacy" | "traits"> &
  Partial<Pick<ProviderModelDto, "capabilities">>;

/** The agent drives everything through tool calls, so a text model without
 * function calling bricks Clovy — prompts run but no file, shell, or memory
 * tool ever executes. The capability name comes from the provider catalog
 * (`supportsFunctionCalling`, derived from the provider's supported
 * parameters); match defensively on the normalized name so a rename to
 * snake_case or "tool calling" keeps working. */
export function modelSupportsTools(
  model: Partial<Pick<ProviderModelDto, "capabilities" | "provider">>,
) {
  // A bring-your-own local endpoint can't be probed for function-calling
  // support from the catalog, and hard-blocking it would make the feature
  // unusable. Tool support is unverifiable and depends on the user's own
  // model, so we treat local as capable and surface a non-blocking caveat in
  // the picker instead of disabling selection.
  if (model.provider === "local") return true;
  return (model.capabilities ?? []).some((capability) => {
    const normalized = capability.toLowerCase().replace(/[^a-z]/g, "");
    return normalized.includes("functioncalling") || normalized.includes("toolcalling");
  });
}

export function modelAvailableForMode(
  mode: ProviderModelMode,
  model: Partial<Pick<ProviderModelDto, "id" | "capabilities" | "provider">>,
) {
  // Text models generate notes; they do not need to call tools.
  if (mode === "generation" && model.id === AUTO_MODEL_ID) return false;
  return true;
}

/** Interface-language display name of each provider a user can save a key for. */
export const BUILTIN_PROVIDER_NAME_KEYS: Record<BuiltinProvider, MessageKey> = {
  groq: "settings.providerKeys.groq.name",
  openrouter: "settings.providerKeys.openrouter.name",
  fal: "settings.providerKeys.fal.name",
  tavily: "settings.providerKeys.tavily.name",
};

/** Whether the model can read image input (vision). Mirrors
 * `modelSupportsTools`: key off the authoritative capability flag on
 * `capabilities` only, never `traits`. The catalog lists a capability only when
 * the provider catalog reports it (image input for `supportsVision`), so
 * `capabilities` reliably lists genuine vision support. `traits` is
 * descriptive text that conflates image OUTPUT with image INPUT — matching it
 * would let the image-attach fallback switch to a model that can't actually
 * read the image. Match defensively on the normalized name so a rename to
 * snake_case keeps working. */
export function modelSupportsImageInput(model: Partial<Pick<ProviderModelDto, "capabilities">>) {
  return (model.capabilities ?? []).some((capability) => {
    const normalized = capability.toLowerCase().replace(/[^a-z]/g, "");
    return normalized.includes("supportsvision");
  });
}

// Strongest claim wins: E2EE models are also private, but "encrypted into the
// enclave" is the property worth surfacing.
export function modelPrivacyBadge(
  model: ModelPrivacySignals,
  flags = modelPrivacyFlags(model),
): ModelPrivacyBadge | undefined {
  if (flags.e2ee) {
    return {
      mode: "e2ee",
      label: t("lib.modelPrivacy.e2eeLabel"),
      description: t("lib.modelPrivacy.e2eeDescription"),
    };
  }
  if (flags.private) {
    return {
      mode: "private",
      label: t("lib.modelPrivacy.privateLabel"),
      description: t("lib.modelPrivacy.privateDescription"),
    };
  }
  if (flags.anonymous) {
    return {
      mode: "anonymous",
      label: t("lib.modelPrivacy.anonymousLabel"),
      description: t("lib.modelPrivacy.anonymousDescription"),
    };
  }
  return undefined;
}

/** What the catalog's "Private" filter keeps: zero-retention or stronger.
 * E2EE models are also private, so they pass; anonymized models (prompts may
 * be retained) do not. A loopback local model never leaves the machine —
 * stronger than any zero-retention claim — so it passes too (a non-loopback
 * custom endpoint reports "external" and makes no claim). */
export function modelIsPrivate(model: ModelPrivacySignals) {
  if ((model.privacy ?? "").toLowerCase() === "local") return true;
  const flags = modelPrivacyFlags(model);
  return flags.e2ee || flags.private;
}

export function modelPrivacyFlags(model: ModelPrivacySignals): ModelPrivacyFlags {
  const privacy = (model.privacy ?? "").toLowerCase();
  const traits = model.traits.map((trait) => trait.toLowerCase());
  const capabilities = (model.capabilities ?? []).map((capability) => capability.toLowerCase());
  return {
    e2ee:
      privacy === "e2ee" ||
      traits.some((trait) => trait === "e2ee") ||
      capabilities.some((capability) => capability === "e2ee"),
    private: privacy === "private" || traits.some((trait) => trait === "private"),
    anonymous:
      privacy.includes("anonymous") ||
      privacy.includes("anonymized") ||
      traits.some((trait) => trait.includes("anonymous") || trait.includes("anonymized")),
    uncensored: traits.some((trait) => trait.includes("uncensored")),
  };
}
