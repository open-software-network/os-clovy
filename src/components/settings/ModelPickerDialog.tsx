import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconFire1 } from "central-icons/IconFire1";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  modelAvailableForMode,
  modelPrivacyBadge,
  modelPrivacyFlags,
} from "../../lib/model-privacy";
import { formatNumber, isEnglishLocale, t as translate, useT } from "../../i18n";
import { pricingLabel, tokenPricingLabels } from "../../lib/model-pricing";
import { suggestedModelsForMode } from "../../lib/suggested-models";
import type { ProviderModelMode, ProviderModelDto } from "../../lib/tauri";
import { Dialog } from "../ui/Dialog";
import { HoverTip } from "../ui/HoverTip";
import { ModelPrivacyChip } from "../ui/ModelPrivacyChip";
import { ProviderLogo } from "./ProviderLogo";

export const AUTO_MODEL_ID = "open-software/auto";

type ModelPickerEntry = {
  key: string;
  model: ProviderModelDto;
  reason?: string;
};

// Model catalog UI shared between Settings (the Models tab rows) and the
// agent workspace (the session bar's model pill): the picker dialog itself
// plus the meta line and option-list helpers it renders with. Moved out of
// AppSettings.tsx verbatim so the workspace can open the picker without
// pulling in the whole settings surface.

export function ModelMeta({ model }: { model: ProviderModelDto }) {
  const t = useT();
  const flags = modelPrivacyFlags(model);
  const privacyBadge = modelPrivacyBadge(model, flags);
  const context = contextLabel(model);
  const price = pricingLabel(model);
  const items: ReactNode[] = [];
  if (price) items.push(<span className="model-meta-price">{price}</span>);
  if (context) items.push(<span>{context}</span>);
  if ((model.modelType === "image" || model.modelType === "video") && model.description) {
    items.push(<span>{model.description}</span>);
  }
  if (privacyBadge) {
    const imagePrivacyLabel =
      model.modelType === "image"
        ? privacyBadge.mode === "private"
          ? t("settingsPanels.models.private")
          : privacyBadge.mode === "anonymous"
            ? t("settingsPanels.models.anonymized")
            : undefined
        : undefined;
    items.push(<ModelPrivacyChip badge={privacyBadge} label={imagePrivacyLabel} />);
  }
  if (flags.uncensored) {
    items.push(
      <span className="model-trait-icon" title={t("settingsPanels.models.uncensored")}>
        <IconFire1 size={14} />
        <span>{t("settingsPanels.models.uncensored")}</span>
      </span>,
    );
  }
  if (items.length === 0) {
    items.push(<span>{t("settingsPanels.models.detailsUnavailable")}</span>);
  }
  return (
    <span className="model-meta-items">
      {items.map((item, index) => (
        <span className="model-meta-item" key={index}>
          {index > 0 ? (
            <span className="model-meta-sep" aria-hidden>
              ·
            </span>
          ) : null}
          {item}
        </span>
      ))}
    </span>
  );
}

export function ModelPickerDialog({
  open,
  mode,
  value,
  options,
  search,
  onSearchChange,
  onClose,
  onSelect,
}: {
  open: boolean;
  mode: ProviderModelMode;
  value: string;
  options: ProviderModelDto[];
  search: string;
  onSearchChange: (value: string) => void;
  onClose: () => void;
  onSelect: (modelId: string) => void;
}) {
  const t = useT();
  // "Suggested" leads with the few models we actually recommend (benchmarks,
  // price, tool use, privacy — see SUGGESTED_MODELS); "All" is the full
  // catalog. Suggested is the default on every open; typing a search always
  // looks across the whole catalog, since three curated rows aren't worth
  // searching.
  const [tab, setTab] = useState<"suggested" | "all">("suggested");
  useEffect(() => {
    if (open) setTab("suggested");
  }, [open, mode]);
  const availableOptions = useMemo(
    () => options.filter((model) => modelAvailableForMode(mode, model)),
    [mode, options],
  );
  const suggested = useMemo(
    () => suggestedModelsForMode(mode, availableOptions),
    [mode, availableOptions],
  );
  const query = search.trim().toLowerCase();
  const searching = query.length > 0;
  const filteredOptions = useMemo<ModelPickerEntry[]>(() => {
    if (searching) {
      return availableOptions
        .filter((model) =>
          [model.name, model.id, model.description, model.privacy, ...model.traits]
            .filter(Boolean)
            .join(" ")
            .toLowerCase()
            .includes(query),
        )
        .map((model) => ({ key: model.id, model }));
    }
    // No suggestions in the catalog (drift, still loading): the empty tab
    // would read as "no models", so fall through to the full list.
    if (tab === "suggested" && suggested.length > 0) {
      return suggested;
    }
    return availableOptions.map((model) => ({ key: model.id, model }));
  }, [availableOptions, query, searching, suggested, tab]);
  const showReasons = !searching && tab === "suggested" && suggested.length > 0;
  const title =
    mode === "transcription"
      ? t("settingsPanels.models.transcriptionModel")
      : mode === "image"
        ? t("settingsPanels.models.imageModel")
        : mode === "video"
          ? t("settingsPanels.models.videoModel")
          : t("settingsPanels.models.textModel");

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      width={760}
      className="model-picker-dialog"
      initialFocusSelector=".model-picker-search"
    >
      <label className="model-picker-search">
        <IconMagnifyingGlass size={15} />
        <input
          className="model-picker-search-input"
          value={search}
          onChange={(event) => onSearchChange(event.currentTarget.value)}
          placeholder={t("settingsPanels.models.search")}
          aria-label={t("settingsPanels.models.search")}
        />
      </label>
      {!searching && suggested.length > 0 ? (
        <div
          className="model-picker-tabs"
          role="tablist"
          aria-label={t("settingsPanels.models.groups")}
        >
          <button
            type="button"
            role="tab"
            aria-selected={tab === "suggested"}
            onClick={() => setTab("suggested")}
          >
            {t("settingsPanels.models.suggested")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "all"}
            onClick={() => setTab("all")}
          >
            {t("settingsPanels.models.all")}
          </button>
        </div>
      ) : null}
      <div className="model-picker-list" role="listbox" aria-label={title}>
        {filteredOptions.map((item) => {
          const { model } = item;
          const selected = model.id === value;
          const reason = showReasons ? item.reason : undefined;
          // A local (bring-your-own) endpoint is selectable, but its tool
          // support can't be verified from here, so it carries a non-blocking
          // caveat rather than the disabling no-tools treatment.
          const localCaveat = mode === "generation" && model.provider === "local";
          return (
            <button
              key={item.key}
              type="button"
              className="model-picker-option"
              role="option"
              aria-selected={selected}
              data-selected={selected}
              onClick={() => onSelect(model.id)}
            >
              <span className="model-picker-logo" aria-hidden>
                <ProviderLogo provider={model.provider} id={model.id} name={model.name} />
              </span>
              <span className="model-picker-name" title={model.description}>
                {model.name}
              </span>
              <span className="model-picker-selected" aria-hidden>
                {selected ? <IconCheckmark2Small size={14} /> : null}
              </span>
              <span className="model-picker-meta">
                {localCaveat ? (
                  <HoverTip
                    tip={t("settingsPanels.models.toolsCaveatTip")}
                    className="model-picker-tools-caveat"
                    compact
                    width={220}
                    tabIndex={0}
                    aria-label={t("settingsPanels.models.toolsCaveatAria")}
                    onClick={(event) => event.stopPropagation()}
                  >
                    {t("settingsPanels.models.toolsNotVerified")}
                  </HoverTip>
                ) : null}
                <ModelMeta model={model} />
              </span>
              {reason ? <span className="model-picker-reason">{reason}</span> : null}
            </button>
          );
        })}
      </div>
    </Dialog>
  );
}

export function selectedModel(options: ProviderModelDto[], value: string) {
  return (
    options.find((model) => model.id === value) ?? {
      provider: "",
      id: value,
      name: value,
      modelType: "",
      traits: [],
      capabilities: [],
    }
  );
}

export function contextLabel(model: ProviderModelDto) {
  if (!model.contextTokens) return undefined;
  if (model.contextTokens >= 1_000_000) {
    return translate("settingsPanels.models.contextMillions", {
      value: trimNumber(model.contextTokens / 1_000_000),
    });
  }
  if (model.contextTokens >= 1_000) {
    return translate("settingsPanels.models.contextThousands", {
      value: trimNumber(model.contextTokens / 1_000),
    });
  }
  return translate("settingsPanels.models.contextTokens", { value: String(model.contextTokens) });
}

/** One decimal at most: "1.5M" in English (as before), the interface locale's digits elsewhere ("1,5M"). */
function trimNumber(value: number) {
  if (Number.isInteger(value)) return String(value);
  if (isEnglishLocale()) return value.toFixed(1);
  return formatNumber(value, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
    useGrouping: false,
  });
}

// The hover card's spec list: the pricing/context facts split into label/value
// pairs so the card renders a compact spec table instead of a prose sentence.
// Falls back to a single "Pricing" row when there is no clean input/output
// USD token-price split.
export function modelSpecEntries(model: ProviderModelDto): { label: string; value: string }[] {
  const entries: { label: string; value: string }[] = [];
  const prices = tokenPricingLabels(model);
  if (prices) {
    entries.push({ label: translate("settingsPanels.models.input"), value: `${prices.input} /1M` });
    entries.push({
      label: translate("settingsPanels.models.output"),
      value: `${prices.output} /1M`,
    });
  } else {
    const price = pricingLabel(model);
    if (price) entries.push({ label: translate("settingsPanels.models.pricing"), value: price });
  }
  if (model.contextTokens) {
    const context =
      model.contextTokens >= 1_000_000
        ? translate("settingsPanels.models.tokensMillions", {
            value: trimNumber(model.contextTokens / 1_000_000),
          })
        : model.contextTokens >= 1_000
          ? translate("settingsPanels.models.tokensThousands", {
              value: trimNumber(model.contextTokens / 1_000),
            })
          : translate("settingsPanels.models.tokensCount", { value: String(model.contextTokens) });
    entries.push({ label: translate("settingsPanels.models.context"), value: context });
  }
  return entries;
}

export function modelOptions(models: ProviderModelDto[], selectedModel: string) {
  const modelId = selectedModel.trim();
  const namedModels = models.filter((model) => model.id !== AUTO_MODEL_ID);
  if (!modelId || modelId === AUTO_MODEL_ID || namedModels.some((model) => model.id === modelId)) {
    return namedModels;
  }
  return [
    {
      provider: "",
      id: modelId,
      name: modelId,
      modelType: "",
      traits: [],
      capabilities: [],
    },
    ...namedModels,
  ];
}
