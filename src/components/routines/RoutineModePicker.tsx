import { IconShieldCheck } from "central-icons/IconShieldCheck";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { type TFunction, useT } from "../../i18n";
import { SegmentedControl } from "../ui/SegmentedControl";

function modeOptions(t: TFunction) {
  return [
    {
      value: "sandboxed",
      label: (
        <>
          <IconShieldCheck size={14} aria-hidden />
          {t("routines.mode.sandboxed")}
        </>
      ),
      ariaLabel: t("routines.mode.sandboxed"),
    },
    {
      value: "unrestricted",
      label: (
        <>
          <IconShieldCrossed size={14} aria-hidden />
          {t("routines.mode.unrestricted")}
        </>
      ),
      ariaLabel: t("routines.mode.unrestricted"),
    },
  ] as const;
}

/** The per-routine sandbox choice. Like the chat picker, Unrestricted is a
 * deliberate opt-in per routine, never a sticky preference. */
export function RoutineModePicker({
  unrestricted,
  onChange,
}: {
  unrestricted: boolean;
  onChange: (unrestricted: boolean) => void;
}) {
  const t = useT();
  return (
    <>
      <SegmentedControl
        value={unrestricted ? "unrestricted" : "sandboxed"}
        onValueChange={(value) => onChange(value === "unrestricted")}
        options={modeOptions(t)}
        // The indicator goes terracotta while Unrestricted is armed, same
        // warm accent as the composer's sandbox trigger.
        className={unrestricted ? "segmented-warm" : undefined}
        aria-label={t("routines.mode.question")}
      />
      <p className="routines-mode-hint">
        {unrestricted ? t("routines.mode.unrestrictedHint") : t("routines.mode.sandboxedHint")}
      </p>
    </>
  );
}
