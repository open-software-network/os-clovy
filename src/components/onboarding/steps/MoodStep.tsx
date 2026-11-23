import { useState } from "react";
import {
  ONBOARDING_MOOD_PERSONALITY_PRESETS,
  ONBOARDING_MOODS,
  ONBOARDING_AREA_MOOD_PRESETS,
  onboardingMood,
  saveOnboardingMood,
  type OnboardingArea,
  type OnboardingMood,
} from "../../../lib/onboarding";
import { useT } from "../../../i18n";
import { setClovyPersona } from "../../../lib/tauri";
import { OnboardingCharacter, ONBOARDING_MOOD_PRESENTATION } from "../OnboardingCharacter";
import { StepActions, StepCard } from "../StepChrome";

export function MoodStep({
  area,
  onContinue,
}: {
  area: OnboardingArea;
  onContinue: (mood: OnboardingMood) => void;
}) {
  const t = useT();
  const [selected, setSelected] = useState<OnboardingMood>(() =>
    onboardingMood(ONBOARDING_AREA_MOOD_PRESETS[area]),
  );
  const [focusVisible, setFocusVisible] = useState<OnboardingMood | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const selectedPresentation = ONBOARDING_MOOD_PRESENTATION[selected];

  async function continueWithMood() {
    setSaving(true);
    setSaveError(null);
    try {
      await setClovyPersona({ area, ...ONBOARDING_MOOD_PERSONALITY_PRESETS[selected] });
      saveOnboardingMood(selected);
      onContinue(selected);
    } catch {
      setSaveError(t("onboarding.mood.saveError"));
    } finally {
      setSaving(false);
    }
  }

  return (
    <StepCard
      title={t("onboarding.mood.title")}
      subtitle={t("onboarding.mood.subtitle")}
      wide
      className="onboarding-card-moods"
    >
      <div className="onboarding-personality-stage" data-mood={selected}>
        <OnboardingCharacter mood={selected} className="onboarding-personality-character" />
        <div className="onboarding-personality-stage-copy" aria-hidden="true">
          <p className="onboarding-personality-greeting">{selectedPresentation.greeting}</p>
        </div>
        <fieldset className="onboarding-personality-picker">
          <legend className="visually-hidden">{t("onboarding.mood.legend")}</legend>
          <div className="onboarding-personality-options">
            {ONBOARDING_MOODS.map((mood) => {
              const active = mood === selected;
              const { label, description } = ONBOARDING_MOOD_PRESENTATION[mood];
              return (
                <label
                  key={mood}
                  className="onboarding-personality-option"
                  data-selected={active ? "true" : undefined}
                  data-focus-visible={focusVisible === mood ? "true" : undefined}
                >
                  <input
                    className="visually-hidden"
                    type="radio"
                    name="onboarding-mood"
                    value={mood}
                    checked={active}
                    onChange={() => {
                      setSelected(mood);
                      setSaveError(null);
                    }}
                    onFocus={(event) => {
                      setFocusVisible(event.currentTarget.matches(":focus-visible") ? mood : null);
                    }}
                    onBlur={() => setFocusVisible(null)}
                  />
                  <span className="onboarding-personality-option-name">{label}</span>
                  <span className="onboarding-personality-option-description">{description}</span>
                </label>
              );
            })}
          </div>
        </fieldset>
      </div>
      <p className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
        {t("onboarding.mood.status", {
          mood: selectedPresentation.label,
          greeting: selectedPresentation.greeting,
        })}
      </p>
      {saveError ? (
        <p className="onboarding-mood-save-error" role="alert">
          {saveError}
        </p>
      ) : null}
      <StepActions
        continueLabel={saving ? t("common.saving") : t("common.continue")}
        continueDisabled={saving}
        onContinue={() => void continueWithMood()}
      />
    </StepCard>
  );
}
