import { type MessageKey, t } from "../i18n/translate";
import type { ProviderModelDto } from "./tauri";

// Clovy's default image model. Mirrors DEFAULT_IMAGE_MODEL in the Rust providers
// module (src-tauri/src/providers/mod.rs) — keep the two in sync.
export const DEFAULT_IMAGE_MODEL = "fal-ai/flux/dev";

type ImageModelDefinition = Pick<ProviderModelDto, "id" | "name"> & {
  /** Catalog key for the description, read in the interface language. */
  descriptionKey: MessageKey;
};

function imageModel({ descriptionKey, ...model }: ImageModelDefinition): ProviderModelDto {
  const entry: ProviderModelDto = {
    provider: "fal",
    modelType: "image",
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

// Curated fal models with translated descriptions. A key is needed to generate,
// not to choose a model. Keep IDs in sync with KNOWN_IMAGE_MODELS.
export const IMAGE_MODELS: ProviderModelDto[] = [
  imageModel({
    id: DEFAULT_IMAGE_MODEL,
    name: "FLUX.1 [dev]",
    descriptionKey: "lib.imageModels.fluxDev",
  }),
  imageModel({
    id: "fal-ai/flux/schnell",
    name: "FLUX.1 [schnell]",
    descriptionKey: "lib.imageModels.fluxSchnell",
  }),
  imageModel({
    id: "fal-ai/qwen-image",
    name: "Qwen Image",
    descriptionKey: "lib.imageModels.qwenImage",
  }),
];

export function imageModelCatalog(): ProviderModelDto[] {
  return IMAGE_MODELS;
}
