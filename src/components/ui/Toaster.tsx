import { IconCheckCircle2 } from "central-icons-filled/IconCheckCircle2";
import { IconCircleInfo } from "central-icons-filled/IconCircleInfo";
import { IconExclamationCircle } from "central-icons-filled/IconExclamationCircle";
import { IconExclamationTriangle } from "central-icons-filled/IconExclamationTriangle";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { Toaster as SonnerToaster } from "sonner";
import { useT } from "../../i18n";
import { DotSpinner } from "../DotSpinner";

// The Clovy-styled wrapper around sonner's toaster. Sonner ships as an unstyled
// primitive here (toastOptions.unstyled) — every visual comes from the
// `.clovy-toast*` rules in styles/app.css, so toasts inherit the design tokens
// and follow all five theme presets plus dark mode automatically. The tone
// icons are the filled central-icons-filled set (never sonner's bundled
// Lucide-style set); the close glyph stays outline and the loading glyph is
// Clovy's own dot spinner.
//
// `toast` is re-exported straight from sonner so callers fire notifications the
// familiar way: `toast("Saved")`, `toast.success(...)`, `toast.error(...)`,
// `toast.info(...)`, `toast.warning(...)`, `toast.loading(...)`, or
// `toast(message, { action: { label, onClick } })`.
export { toast } from "sonner";

/** The tones Clovy styles. Mirrors the subset of sonner's toast types we use. */
export type ToastTone = "default" | "success" | "error" | "info" | "warning" | "loading";

const TONE_ICON_SIZE = 16;

/**
 * Mount this once in the app shell. Toasts render into a portal at
 * document.body, anchored to the top-right corner of the content panel so they
 * read as messages tucked into the upper-right of the workspace.
 */
export function Toaster() {
  const t = useT();
  return (
    <SonnerToaster
      position="top-right"
      containerAriaLabel={t("shell.toast.containerLabel")}
      // Clear the custom titlebar and use the same top/right inset from the
      // workspace corner.
      offset={{ top: "calc(var(--titlebar-h) + var(--sp-6))", right: "var(--sp-6)" }}
      gap={8}
      // Slightly calmer than sonner's 4s default without lingering. Loading
      // toasts still persist until resolved or dismissed.
      duration={5000}
      closeButton
      // Decorative glyphs — the toast text carries the meaning for assistive
      // tech, and sonner marks the toast itself with the right role/aria-live.
      icons={{
        success: <IconCheckCircle2 size={TONE_ICON_SIZE} ariaHidden />,
        error: <IconExclamationCircle size={TONE_ICON_SIZE} ariaHidden />,
        warning: <IconExclamationTriangle size={TONE_ICON_SIZE} ariaHidden />,
        info: <IconCircleInfo size={TONE_ICON_SIZE} ariaHidden />,
        loading: <DotSpinner />,
        close: <IconCrossSmall size={16} ariaHidden />,
      }}
      toastOptions={{
        unstyled: true,
        closeButtonAriaLabel: t("shell.toast.close"),
        classNames: {
          toast: "clovy-toast",
          content: "clovy-toast-content",
          title: "clovy-toast-title",
          description: "clovy-toast-description",
          icon: "clovy-toast-icon",
          loader: "clovy-toast-loader",
          actionButton: "btn btn-secondary clovy-toast-action",
          cancelButton: "btn btn-secondary clovy-toast-cancel",
          closeButton: "icon-button clovy-toast-close",
          default: "clovy-toast--default",
          success: "clovy-toast--success",
          error: "clovy-toast--error",
          info: "clovy-toast--info",
          warning: "clovy-toast--warning",
          loading: "clovy-toast--loading",
        },
      }}
    />
  );
}
