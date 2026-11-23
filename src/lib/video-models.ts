import { type MessageKey, t } from "../i18n/translate";
import type { ProviderModelDto } from "./tauri";

// Clovy's default video model. Mirrors DEFAULT_VIDEO_MODEL in the Rust providers
// module (src-tauri/src/providers/mod.rs). Keep the two in sync.
export const DEFAULT_VIDEO_MODEL = "fal-ai/wan/v2.2-a14b/text-to-video";

type VideoModelDefinition = Pick<ProviderModelDto, "id" | "name"> & {
  /** Catalog key for the description, read in the interface language. */
  descriptionKey: MessageKey;
};

function videoModel({ descriptionKey, ...model }: VideoModelDefinition): ProviderModelDto {
  const entry: ProviderModelDto = {
    provider: "fal",
    modelType: "video",
    capabilities: [],
    traits: [],
    ...model,
  };
  Object.defineProperty(entry, "description", {
    enumerable: true,
    configurable: true,
    get: () => t(descriptionKey),
  });
  return entry;
}

// Curated fal text-to-video endpoints. Keep IDs in sync with KNOWN_VIDEO_MODELS
// in the native providers module. The direct fal adapter validates supported
// duration, resolution and aspect ratio before submitting a job. Choosing a
// model needs no key; generation requires the user's fal key.
// No retention claim is made for the provider.
export const VIDEO_MODELS: ProviderModelDto[] = [
  videoModel({
    id: DEFAULT_VIDEO_MODEL,
    name: "Wan 2.2 A14B",
    descriptionKey: "lib.videoModels.wan22",
  }),
  videoModel({
    id: "alibaba/wan-3.0/text-to-video",
    name: "Wan 3.0",
    descriptionKey: "lib.videoModels.wan30",
  }),
];

export function videoModelCatalog(): ProviderModelDto[] {
  return VIDEO_MODELS;
}
