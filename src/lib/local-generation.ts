import { t } from "../i18n/translate";
import type { LocalGenerationSettingsDto, ProviderModelDto } from "./tauri";

// Bring-your-own local text generation. The model catalog is derived
// client-side from provider settings, so the user's configured local endpoint
// is surfaced as a synthetic catalog option. These helpers are shared between
// the settings surface and the agent composer, so they live outside
// AppSettings.

export const LOCAL_GENERATION_OPTION_ID_PREFIX = "__june_local_generation__:";

/** Stable synthetic id for the local model catalog option. Prefixed so it can
 * never collide with a real remote model id (finding: a raw local id that
 * matched a remote id let the picker persist it as the remote model). */
export function localGenerationOptionId(modelId: string) {
  return `${LOCAL_GENERATION_OPTION_ID_PREFIX}${encodeURIComponent(modelId.trim())}`;
}

/** Inverse of {@link localGenerationOptionId}: the raw local model id encoded
 * in a synthetic option id, or null when the id is not a synthetic local
 * option (or is malformed). The tagged id stays intact in session settings to
 * retain upstream provenance; Clovy's on-device integration uses this inverse
 * only when it needs to display or forward the raw local id. A chat engine
 * option also names its endpoint (`<model>@<endpoint id>`, both encoded); see
 * {@link localGenerationEndpointId}. */
export function rawLocalGenerationModelId(optionId: string): string | null {
  if (!optionId.startsWith(LOCAL_GENERATION_OPTION_ID_PREFIX)) return null;
  const [encodedModel] = optionId.slice(LOCAL_GENERATION_OPTION_ID_PREFIX.length).split("@");
  try {
    const decoded = decodeURIComponent(encodedModel ?? "").trim();
    return decoded || null;
  } catch {
    return null;
  }
}

/** The registered endpoint a chat engine option names, or null for the older
 * form that only carries a model id (routed by model). */
export function localGenerationEndpointId(optionId: string): string | null {
  if (!optionId.startsWith(LOCAL_GENERATION_OPTION_ID_PREFIX)) return null;
  const separator = optionId.indexOf("@", LOCAL_GENERATION_OPTION_ID_PREFIX.length);
  if (separator < 0) return null;
  try {
    const decoded = decodeURIComponent(optionId.slice(separator + 1)).trim();
    return decoded || null;
  } catch {
    return null;
  }
}

/** Display-only row for a session whose tagged local choice no longer matches
 * the configured endpoint. Keep the original choice visible; sends fail closed
 * in Clovy's on-device provider proxy until the user reconfigures or selects
 * another model. */
export function unavailableLocalGenerationOption(optionId: string): ProviderModelDto | null {
  const modelId = rawLocalGenerationModelId(optionId);
  if (!modelId) return null;
  return {
    provider: "local",
    id: optionId,
    name: `Local: ${modelId}`,
    modelType: "text",
    description: t("lib.localGeneration.unavailableDescription"),
    pricing: { display: "Local" },
    traits: ["local"],
    capabilities: [],
    priceUnit: "local",
    priceDescription: "Local",
  };
}

/** True when the endpoint resolves to this machine: localhost, any
 * *.localhost name, the 127.0.0.0/8 loopback block, or the IPv6 [::1]
 * literal. Invalid input is treated as non-loopback (returns false) so the
 * caller shows the "leaves your device" warning rather than a false
 * reassurance. */
export function isLoopbackUrl(url: string): boolean {
  let host: string;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return false;
  }
  // URL.hostname keeps the brackets on IPv6 literals ("[::1]").
  const bare = host.replace(/^\[|\]$/g, "");
  if (bare === "localhost" || bare.endsWith(".localhost")) return true;
  if (bare === "::1") return true;
  const octets = bare.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (octets) {
    const parts = octets.slice(1).map(Number);
    if (parts.every((part) => part <= 255) && parts[0] === 127) return true;
  }
  return false;
}

/** Prepends the user's configured local endpoint as a synthetic catalog
 * option when a model id is set. Capabilities are left empty: a local model's
 * tool support can't be verified from here, so it must not be advertised as
 * tool-capable (see modelSupportsTools, which special-cases the local
 * provider instead). Privacy is only claimed as "local" for a loopback
 * endpoint; a remote endpoint is marked "external" since prompts leave the
 * device. */
export function withLocalGenerationOption(
  models: ProviderModelDto[],
  localGeneration: LocalGenerationSettingsDto,
): ProviderModelDto[] {
  const modelId = localGeneration.modelId.trim();
  if (!modelId) return models;
  const loopback = isLoopbackUrl(localGeneration.baseUrl);
  const localModel: ProviderModelDto = {
    provider: "local",
    id: localGenerationOptionId(modelId),
    name: `Local: ${modelId}`,
    modelType: "text",
    description: loopback
      ? "OpenAI-compatible local text model."
      : "OpenAI-compatible text model on a remote endpoint.",
    privacy: loopback ? "local" : "external",
    pricing: { display: "Local" },
    traits: ["local"],
    capabilities: [],
    priceUnit: "local",
    priceDescription: "Local",
  };
  return [localModel, ...models];
}
