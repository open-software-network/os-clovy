import { IconBrainSideview } from "central-icons/IconBrainSideview";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconDices } from "central-icons/IconDices";
import { IconHomeRoundDoor } from "central-icons/IconHomeRoundDoor";
import { IconSuitcaseWork } from "central-icons/IconSuitcaseWork";
import { useState } from "react";
import { type MessageKey, useT } from "../../../i18n";
import {
  ONBOARDING_AREAS,
  ONBOARDING_AREA_MOOD_PRESETS,
  onboardingArea,
  saveOnboardingArea,
  type OnboardingArea,
} from "../../../lib/onboarding";
import { StepActions, StepCard } from "../StepChrome";

const AREA_PRESENTATION: Record<
  OnboardingArea,
  {
    label: MessageKey;
    description: MessageKey;
    icon: typeof IconSuitcaseWork;
  }
> = {
  work: {
    label: "onboarding.area.work.label",
    description: "onboarding.area.work.description",
    icon: IconSuitcaseWork,
  },
  personal: {
    label: "onboarding.area.personal.label",
    description: "onboarding.area.personal.description",
    icon: IconHomeRoundDoor,
  },
  thinking: {
    label: "onboarding.area.thinking.label",
    description: "onboarding.area.thinking.description",
    icon: IconBrainSideview,
  },
  play: {
    label: "onboarding.area.play.label",
    description: "onboarding.area.play.description",
    icon: IconDices,
  },
};

export function AreaStep({ onContinue }: { onContinue: (area: OnboardingArea) => void }) {
  const t = useT();
  const [selected, setSelected] = useState<OnboardingArea | null>(() => onboardingArea());
  const [focusVisible, setFocusVisible] = useState<OnboardingArea | null>(null);

  function continueWithArea() {
    if (!selected) return;
    saveOnboardingArea(selected);
    onContinue(selected);
  }

  return (
    <StepCard
      title={t("onboarding.area.title")}
      subtitle={t("onboarding.area.subtitle")}
      wide
      className="onboarding-card-moods onboarding-card-areas"
    >
      <fieldset className="onboarding-mood-grid onboarding-area-grid">
        <legend className="visually-hidden">{t("onboarding.area.legend")}</legend>
        {ONBOARDING_AREAS.map((area) => {
          const active = area === selected;
          const { label, description, icon: Icon } = AREA_PRESENTATION[area];
          return (
            <label
              key={area}
              className="onboarding-mood-option onboarding-area-option"
              data-area={area}
              data-mood={ONBOARDING_AREA_MOOD_PRESETS[area]}
              data-selected={active ? "true" : undefined}
              data-focus-visible={focusVisible === area ? "true" : undefined}
            >
              <input
                className="visually-hidden"
                type="radio"
                name="onboarding-area"
                value={area}
                checked={active}
                onChange={() => setSelected(area)}
                onFocus={(event) => {
                  setFocusVisible(event.currentTarget.matches(":focus-visible") ? area : null);
                }}
                onBlur={() => setFocusVisible(null)}
              />
              <span className="onboarding-mood-check" aria-hidden data-visible={active}>
                <IconCheckmark2Small size={14} />
              </span>
              <span className="onboarding-area-icon" aria-hidden>
                <Icon size={20} />
              </span>
              <span className="onboarding-mood-copy">
                <span className="onboarding-mood-name">{t(label)}</span>
                <span className="onboarding-mood-description">{t(description)}</span>
              </span>
            </label>
          );
        })}
      </fieldset>
      <StepActions onContinue={continueWithArea} continueDisabled={!selected} />
    </StepCard>
  );
}
