import { IconCrossMedium } from "central-icons/IconCrossMedium";
import { type ReactNode, useEffect, useId, useLayoutEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useT } from "../../i18n";

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  /** Optional element rendered into the header beside the title (e.g. an icon). */
  leading?: ReactNode;
  /** Slot for the form / body content. */
  children: ReactNode;
  /** Slot for buttons; rendered right-aligned by default. */
  footer?: ReactNode;
  /** Disable closing on backdrop click (still closes on Esc). */
  disableBackdropClose?: boolean;
  /** Disable all close affordances (X button, Esc, backdrop) while a
   * consumer-side operation is in flight. The X button renders disabled;
   * Esc and backdrop clicks are ignored. */
  closeDisabled?: boolean;
  /** Disables default focus management when the consumer wants to take over. */
  initialFocusSelector?: string;
  /** Optional width override. Defaults to the comfortable 460px form width. */
  width?: number | string;
  /** Optional class hook for unusual dialogs. */
  className?: string;
};

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Base dialog primitive. Renders into a portal with a blurred backdrop,
 * a centered card, focus trap, and Esc-to-close — the substrate we share
 * across folder create / rename / move and any future dialog.
 */
export function Dialog({
  open,
  onClose,
  title,
  description,
  leading,
  children,
  footer,
  disableBackdropClose = false,
  closeDisabled = false,
  initialFocusSelector,
  width,
  className,
}: DialogProps) {
  const t = useT();
  const cardRef = useRef<HTMLDivElement | null>(null);
  const onCloseRef = useRef(onClose);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const titleId = useId();
  const descriptionId = useId();

  // Keep the latest onClose without making it a useEffect dependency.
  // Re-running the keydown effect on every parent render would refocus
  // `previousFocus` mid-typing and bounce focus out of the dialog.
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);
  // Keep closeDisabled live for the keydown effect without making it a
  // dependency, mirroring the onCloseRef pattern above. Toggling the close
  // lock (e.g. the Notion consent dialog entering/leaving the "waiting for
  // browser" state on OAuth failure) would otherwise tear the keydown
  // effect down and re-run it — churning the listener and refocusing
  // `previousFocus` unnecessarily while the dialog stays open.
  const closeDisabledRef = useRef(closeDisabled);
  useEffect(() => {
    closeDisabledRef.current = closeDisabled;
  }, [closeDisabled]);

  useEffect(() => {
    if (!open) return;
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        if (!closeDisabledRef.current) onCloseRef.current();
        return;
      }
      if (event.key !== "Tab" || !cardRef.current) return;
      const focusables = Array.from(cardRef.current.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = previousOverflow;
      previousFocusRef.current?.focus?.();
    };
  }, [open]);

  useLayoutEffect(() => {
    if (!open || !cardRef.current) return;
    const target = initialFocusSelector
      ? cardRef.current.querySelector<HTMLElement>(initialFocusSelector)
      : cardRef.current.querySelector<HTMLElement>(FOCUSABLE);
    target?.focus();
  }, [open, initialFocusSelector]);

  if (!open) return null;

  return createPortal(
    <div
      className="dialog-backdrop"
      data-open="true"
      onMouseDown={(event) => {
        if (disableBackdropClose) return;
        if (closeDisabled) return;
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={cardRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
        className={`dialog-card${className ? ` ${className}` : ""}`}
        style={width ? { width } : undefined}
      >
        <header className="dialog-header">
          {leading ? <span className="dialog-leading">{leading}</span> : null}
          <h2 id={titleId} className="dialog-title">
            {title}
          </h2>
          <button
            type="button"
            className="dialog-close"
            aria-label={t("common.close")}
            onClick={onClose}
            disabled={closeDisabled}
            aria-disabled={closeDisabled || undefined}
          >
            <IconCrossMedium size={14} />
          </button>
        </header>
        {description ? (
          <p id={descriptionId} className="dialog-description">
            {description}
          </p>
        ) : null}
        {children}
        {footer ? <footer className="dialog-footer">{footer}</footer> : null}
      </div>
    </div>,
    document.body,
  );
}

export function DialogField({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: ReactNode;
  htmlFor?: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="dialog-field">
      <label className="dialog-field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {hint ? <p className="dialog-field-hint">{hint}</p> : null}
    </div>
  );
}
