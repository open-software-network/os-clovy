import { useT } from "../../i18n";
import { DotSpinner } from "../DotSpinner";
import type { SpinnerSize } from "../../lib/spinner-grid";

// The app-wide loading indicator: a neutral dot spinner (see
// DotSpinner). This wrapper owns the accessibility contract — the glyph itself
// is decorative. "sm" (3×3) is the compact default, "md" is a slightly larger
// 3×3 option, and "lg" is the 5×5 board for standalone loading moments.
type SpinnerProps = {
  className?: string;
  size?: SpinnerSize;
  "aria-hidden"?: boolean;
  "aria-label"?: string;
};

export function Spinner({
  className,
  size,
  "aria-hidden": ariaHidden,
  "aria-label": ariaLabel,
}: SpinnerProps) {
  const t = useT();
  const classes = ["spinner", className].filter(Boolean).join(" ");
  return (
    <span
      role={ariaHidden ? undefined : "status"}
      aria-hidden={ariaHidden}
      aria-label={ariaHidden ? undefined : (ariaLabel ?? t("shell.spinner.loading"))}
      className={classes}
    >
      <DotSpinner size={size} />
    </span>
  );
}
