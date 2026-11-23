import { useEffect, useState } from "react";
import {
  dictationSettings,
  listProviderModels,
  setDictationCleanup,
  setDictationHudPlacement,
  setDictationStyle,
} from "../../lib/tauri";
import type { DictationHudPlacement, DictationStyle, ProviderModelDto } from "../../lib/tauri";
import {
  PROVIDER_MODEL_SETTINGS_CHANGED_EVENT,
  type ProviderModelSettingsChangedDetail,
} from "../../lib/model-privacy";
import { SegmentedControl } from "../ui/SegmentedControl";
import { Switch } from "../ui/Switch";
import { type MessageKey, useT } from "../../i18n";

const STYLE_OPTIONS = [
  { value: "standard" as const, labelKey: "settings.style.standard" },
  { value: "casualLowercase" as const, labelKey: "settings.style.casual" },
  { value: "formal" as const, labelKey: "settings.style.formal" },
] satisfies { value: DictationStyle; labelKey: MessageKey }[];

const HUD_PLACEMENT_OPTIONS = [
  { value: "bottom" as const, labelKey: "settings.style.hudPlacementBottom" },
  { value: "top" as const, labelKey: "settings.style.hudPlacementTop" },
] satisfies { value: DictationHudPlacement; labelKey: MessageKey }[];

const SAMPLES: Record<DictationStyle, { description: MessageKey; sample: MessageKey }> = {
  standard: {
    description: "settings.style.standardDescription",
    sample: "settings.style.standardSample",
  },
  casualLowercase: {
    description: "settings.style.casualDescription",
    sample: "settings.style.casualSample",
  },
  formal: {
    description: "settings.style.formalDescription",
    sample: "settings.style.formalSample",
  },
};

type CleanupModelOption = { id: string; name: string };

/** Dictation cleanup is one OpenRouter chat completion: only catalog ids of
 * the form `vendor/model` qualify, never Auto, a CLI, or an endpoint. */
function cleanupModelOptions(models: ProviderModelDto[]): CleanupModelOption[] {
  return models
    .filter(
      (model) =>
        model.provider === "openrouter" &&
        /^[^/\s:]+\/\S+$/.test(model.id) &&
        !model.id.startsWith("__"),
    )
    .map((model) => ({ id: model.id, name: model.name.trim() || model.id }));
}

export function StyleSettingsSection() {
  const t = useT();
  const [style, setStyle] = useState<DictationStyle>("standard");
  const [hudPlacement, setHudPlacement] = useState<DictationHudPlacement>("bottom");
  const [cleanupEnabled, setCleanupEnabled] = useState(false);
  const [cleanupModel, setCleanupModel] = useState("");
  const [models, setModels] = useState<CleanupModelOption[]>([]);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let cancelled = false;
    dictationSettings()
      .then((response) => {
        if (cancelled) return;
        setStyle(response.settings.style);
        setCleanupEnabled(response.settings.cleanupEnabled ?? false);
        setCleanupModel(response.settings.cleanupModel ?? "");
        setHudPlacement(response.settings.hudPlacement ?? "bottom");
      })
      .catch((caught: unknown) => {
        if (!cancelled) setError(messageFromError(caught));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!cleanupEnabled) return;
    let cancelled = false;
    let request = 0;
    const reload = () => {
      const current = ++request;
      listProviderModels("generation")
        .then((response) => {
          if (!cancelled && current === request) setModels(cleanupModelOptions(response.models));
        })
        .catch(() => {
          // The saved cleanup model remains available when the catalog fails.
          if (!cancelled && current === request) setModels([]);
        });
    };
    const changed = (event: Event) => {
      if ((event as CustomEvent<ProviderModelSettingsChangedDetail>).detail.mode === "generation")
        reload();
    };
    reload();
    window.addEventListener(PROVIDER_MODEL_SETTINGS_CHANGED_EVENT, changed);
    return () => {
      cancelled = true;
      window.removeEventListener(PROVIDER_MODEL_SETTINGS_CHANGED_EVENT, changed);
    };
  }, [cleanupEnabled]);

  async function toggleCleanup(enabled: boolean) {
    setCleanupEnabled(enabled);
    try {
      const next = await setDictationCleanup(enabled);
      setCleanupEnabled(next.cleanupEnabled);
      setCleanupModel(next.cleanupModel);
      setError(undefined);
    } catch (caught) {
      setCleanupEnabled(!enabled);
      setError(messageFromError(caught));
    }
  }

  async function selectCleanupModel(model: string) {
    const previous = cleanupModel;
    setCleanupModel(model);
    try {
      const next = await setDictationCleanup(true, model);
      setCleanupModel(next.cleanupModel);
      setError(undefined);
    } catch (caught) {
      setCleanupModel(previous);
      setError(messageFromError(caught));
    }
  }

  async function selectHudPlacement(nextPlacement: DictationHudPlacement) {
    const previous = hudPlacement;
    setHudPlacement(nextPlacement);
    try {
      const next = await setDictationHudPlacement(nextPlacement);
      setHudPlacement(next.hudPlacement ?? nextPlacement);
      setError(undefined);
    } catch (caught) {
      setHudPlacement(previous);
      setError(messageFromError(caught));
    }
  }

  async function selectStyle(nextStyle: DictationStyle) {
    setStyle(nextStyle);
    try {
      const next = await setDictationStyle(nextStyle);
      setStyle(next.style);
      setError(undefined);
    } catch (caught) {
      setError(messageFromError(caught));
    }
  }

  const current = SAMPLES[style];
  const modelOptions =
    cleanupModel && !models.some((model) => model.id === cleanupModel)
      ? [{ id: cleanupModel, name: cleanupModel }, ...models]
      : models;

  return (
    <section className="settings-group" aria-labelledby="style-heading">
      <h2 id="style-heading" className="settings-group-heading">
        {t("settings.style.title")}
      </h2>
      <div className="settings-card">
        <div className="settings-rows">
          <div className="settings-row">
            <div className="settings-row-info">
              <h3 id="dictation-hud-placement-title" className="settings-row-title">
                {t("settings.style.hudPlacement")}
              </h3>
              <p className="settings-row-description">
                {t("settings.style.hudPlacementDescription")}
              </p>
            </div>
            <div className="settings-row-control">
              <SegmentedControl
                value={hudPlacement}
                onValueChange={(value) => void selectHudPlacement(value)}
                options={HUD_PLACEMENT_OPTIONS.map((option) => ({
                  value: option.value,
                  label: t(option.labelKey),
                }))}
                aria-label={t("settings.style.hudPlacement")}
              />
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-info">
              <h3 id="dictation-cleanup-title" className="settings-row-title">
                {t("settings.style.cleanup")}
              </h3>
              <p className="settings-row-description">{t("settings.style.cleanupDescription")}</p>
              {error ? <p className="settings-row-error">{error}</p> : null}
            </div>
            <div className="settings-row-control">
              <Switch
                checked={cleanupEnabled}
                onCheckedChange={(enabled) => void toggleCleanup(enabled)}
                aria-labelledby="dictation-cleanup-title"
              />
            </div>
          </div>
          {cleanupEnabled ? (
            <>
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{t("settings.style.cleanupModel")}</h3>
                  <p className="settings-row-description">
                    {t("settings.style.cleanupModelDescription")}
                  </p>
                </div>
                <div className="settings-row-control">
                  <select
                    className="dialog-input"
                    aria-label={t("settings.style.cleanupModel")}
                    value={cleanupModel}
                    onChange={(event) => void selectCleanupModel(event.target.value)}
                  >
                    {modelOptions.map((model) => (
                      <option key={model.id} value={model.id}>
                        {model.name}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
              <div className="settings-row">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{t("settings.style.outputStyle")}</h3>
                  <p className="settings-row-description">{t(current.description)}</p>
                </div>
                <div className="settings-row-control">
                  <SegmentedControl
                    value={style}
                    onValueChange={(value) => void selectStyle(value)}
                    options={STYLE_OPTIONS.map((option) => ({
                      value: option.value,
                      label: t(option.labelKey),
                    }))}
                    aria-label={t("settings.style.aria")}
                  />
                </div>
              </div>
            </>
          ) : null}
        </div>
        {cleanupEnabled ? (
          <div className="style-preview" aria-live="polite">
            <p className="style-preview-text">{t(current.sample)}</p>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function messageFromError(caught: unknown) {
  if (caught && typeof caught === "object" && "message" in caught) {
    return String((caught as { message: unknown }).message);
  }
  return String(caught);
}
