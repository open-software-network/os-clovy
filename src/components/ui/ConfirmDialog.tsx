import type { ReactNode } from "react";
import { useState } from "react";
import { useT } from "../../i18n";
import { Dialog } from "./Dialog";

type ConfirmDialogProps = {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void | Promise<unknown>;
  title: ReactNode;
  description?: ReactNode;
  confirmLabel?: string;
  /** Label shown on the confirm button while onConfirm is in flight, for
   * consequential actions whose pending state should read as progress
   * ("Upgrading...") rather than a frozen button. */
  confirmBusyLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
};

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  description,
  confirmLabel,
  confirmBusyLabel,
  cancelLabel,
  destructive = false,
}: ConfirmDialogProps) {
  const t = useT();
  const [submitting, setSubmitting] = useState(false);

  async function handleConfirm() {
    if (submitting) return;
    setSubmitting(true);
    try {
      await onConfirm();
      onClose();
    } catch {
      // Keep the dialog open so callers can surface the error and retry.
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={() => {
        if (!submitting) onClose();
      }}
      title={title}
      description={description}
      width={420}
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose} disabled={submitting}>
            {cancelLabel ?? t("common.cancel")}
          </button>
          <button
            type="button"
            className={`primary-action primary-solid${destructive ? " primary-destructive" : ""}`}
            onClick={() => void handleConfirm()}
            disabled={submitting}
            aria-busy={submitting || undefined}
          >
            {submitting && confirmBusyLabel
              ? confirmBusyLabel
              : (confirmLabel ?? t("common.confirm"))}
          </button>
        </>
      }
    >
      <div />
    </Dialog>
  );
}
