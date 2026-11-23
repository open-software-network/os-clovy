import type { ReactNode } from "react";
import { useT } from "../../i18n";
import { ClovyAppTile } from "../brand/ClovyLogo";
import { BrandPrimaryButton } from "../ui/BrandPrimaryButton";

/**
 * One onboarding screen = one welcome-card: a serif title, at most one muted
 * line, then whatever the step needs. The Clovy mark introduces the brand on the first
 * screen only; after that the type carries it.
 */
export function StepCard({
  title,
  subtitle,
  mark,
  wide,
  className,
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  /** Show the Clovy mark above the title (the welcome screen). */
  mark?: boolean;
  /** Steps with a demo card or timeline get a little more room. */
  wide?: boolean;
  /** Extra class on the card for step-specific layout (e.g. the welcome grid). */
  className?: string;
  children?: ReactNode;
}) {
  return (
    <section
      className={`welcome-card onboarding-card${wide ? " wide-card" : ""}${
        className ? ` ${className}` : ""
      }`}
    >
      {mark ? <ClovyAppTile className="welcome-brand-mark" /> : null}
      <h1 className="welcome-title">{title}</h1>
      {subtitle ? <p className="welcome-subtitle">{subtitle}</p> : null}
      {children}
    </section>
  );
}

/**
 * Footer action: one full-width primary button (the gates' pattern), with an
 * optional quiet skip beneath. Never two competing buttons.
 */
export function StepActions({
  continueLabel,
  continueDisabled,
  onContinue,
  onSkip,
  skipLabel,
}: {
  continueLabel?: string;
  continueDisabled?: boolean;
  onContinue: () => void;
  onSkip?: () => void;
  skipLabel?: string;
}) {
  const t = useT();
  return (
    <div className="welcome-providers">
      <BrandPrimaryButton disabled={continueDisabled} onClick={onContinue}>
        {continueLabel ?? t("common.continue")}
      </BrandPrimaryButton>
      {onSkip ? (
        <button type="button" className="onboarding-skip" onClick={onSkip}>
          {skipLabel ?? t("onboarding.skipForNow")}
        </button>
      ) : null}
    </div>
  );
}

export { BrandPrimaryButton as OnboardingPrimaryButton };
