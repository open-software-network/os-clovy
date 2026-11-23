import { type MessageKey, t } from "../i18n/translate";
import { modelSupportsImageInput, modelSupportsTools } from "./model-privacy";
import type { ProviderModelMode, ProviderModelDto } from "./tauri";

export type SuggestedModel = {
  id: string;
  /** Picker-only label. The persisted provider model id is unchanged. */
  label?: string;
  /** One-line "when to choose it" guidance shown in the suggested model card. */
  reason: string;
};

// Historical image attachment recovery uses a concrete vision model.
const PREFERRED_VISION_FALLBACK_IDS = ["moonshotai/kimi-k2.6"];

/**
 * Curated picks for the model picker's "Suggested" tab — the handful of
 * models we actually recommend, weighed on benchmark performance, price, and
 * tool use. Clovy's agent needs tool calling, so every text pick supports
 * tools. No pick claims a privacy property: the catalog reports none for
 * these providers.
 *
 */
export const SUGGESTED_MODELS: Record<ProviderModelMode, SuggestedModel[]> = {
  generation: [
    suggestion("z-ai/glm-5.2", "lib.suggested.glm52"),
    suggestion("moonshotai/kimi-k3", "lib.suggested.kimiK3"),
    suggestion("moonshotai/kimi-k2.6", "lib.suggested.kimiK26"),
    suggestion("z-ai/glm-5.1", "lib.suggested.glm51"),
  ],
  transcription: [
    suggestion("whisper-large-v3-turbo", "lib.suggested.whisperTurbo"),
    suggestion("whisper-large-v3", "lib.suggested.whisper"),
  ],
  // Image models come from the curated local list in lib/image-models.ts, not
  // a fetched catalog; these picks are the shortlist the picker surfaces above
  // "All models". The default (DEFAULT_IMAGE_MODEL, fal-ai/flux/dev) stays
  // first so it is always visible without expanding.
  image: [
    suggestion("fal-ai/flux/dev", "lib.suggested.fluxDev"),
    suggestion("fal-ai/flux/schnell", "lib.suggested.fluxSchnell"),
    suggestion("fal-ai/qwen-image", "lib.suggested.qwenImage"),
  ],
  // Video is a small curated local list (lib/video-models.ts); both models are
  // suggested so they show without expanding "All models". The default
  // (DEFAULT_VIDEO_MODEL, Wan 2.2 A14B) stays first.
  video: [
    suggestion("fal-ai/wan/v2.2-a14b/text-to-video", "lib.suggested.wan22"),
    suggestion("alibaba/wan-3.0/text-to-video", "lib.suggested.wan30"),
  ],
};

function suggestion(id: string, reasonKey: MessageKey): SuggestedModel {
  return {
    id,
    get reason() {
      return t(reasonKey);
    },
  };
}

// The default concrete text model when the catalog is empty or still
// loading: the leading curated pick, which mirrors DEFAULT_GENERATION_MODEL
// in the Rust providers module — the factory default is safe to select
// sight-unseen (the pill renders a name-only stub until the catalog arrives).
export const DEFAULT_GENERATION_SUGGESTION_ID = SUGGESTED_MODELS.generation[0].id;

/**
 * The model Clovy switches to when the user attaches an image while a
 * non-vision model is active. The switch must land on a model that can both
 * read images AND run tools — a vision model without function calling would
 * brick the agent the same way the model picker guards against — so we filter
 * on both capabilities. Among the eligible models we prefer a concrete
 * vision pick (currently Kimi K2.6), so the one-tap fix
 * lands on a sensible default instead of the alphabetically-first vision model
 * (which is otherwise arbitrary — the catalog sorts by display name). If no
 * preferred model is eligible we fall back to the first eligible catalog model.
 * A retired preference is ignored because it is resolved against the live catalog.
 */
export function preferredVisionFallbackModel(
  models: ProviderModelDto[],
): ProviderModelDto | undefined {
  const eligible = models.filter(
    (model) => modelSupportsImageInput(model) && modelSupportsTools(model),
  );
  const preferred = PREFERRED_VISION_FALLBACK_IDS.map((id) =>
    eligible.find((model) => model.id === id),
  ).find((model): model is ProviderModelDto => model !== undefined);
  return preferred ?? eligible[0];
}

/** The curated picks that are actually present in the live catalog, in
 * curated order. Picker-only labels and guidance replace the generic catalog
 * name and description without changing the persisted model id. */
export function suggestedModelsForMode(
  mode: ProviderModelMode,
  options: ProviderModelDto[],
): Array<{
  key: string;
  model: ProviderModelDto;
  reason: string;
}> {
  return SUGGESTED_MODELS[mode].flatMap((suggested, index) => {
    const model = options.find((option) => option.id === suggested.id);
    if (!model) return [];
    return [
      {
        key: `${suggested.id}:${index}`,
        model: {
          ...model,
          ...(suggested.label ? { name: suggested.label, description: suggested.reason } : {}),
        },
        reason: suggested.reason,
      },
    ];
  });
}
