import { useEffect, useId, useState } from "react";
import { useT } from "../../i18n";
import { Dialog, DialogField } from "../ui/Dialog";

type RenameSessionDialogProps = {
  open: boolean;
  currentName: string;
  onClose: () => void;
  onRename: (name: string) => void;
};

export function RenameSessionDialog({
  open,
  currentName,
  onClose,
  onRename,
}: RenameSessionDialogProps) {
  const t = useT();
  const [name, setName] = useState(currentName);
  const formId = useId();
  const inputId = useId();

  useEffect(() => {
    if (!open) return;
    setName(currentName);
  }, [currentName, open]);

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    if (trimmed !== currentName) onRename(trimmed);
    onClose();
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("agent.rename.title")}
      initialFocusSelector="[data-rename-session-input]"
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            form={formId}
            className="primary-action primary-solid"
            disabled={name.trim().length === 0}
          >
            {t("common.rename")}
          </button>
        </>
      }
    >
      <form id={formId} className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("agent.rename.nameLabel")} htmlFor={inputId}>
          <input
            id={inputId}
            data-rename-session-input=""
            className="dialog-input"
            aria-label={t("agent.rename.nameAria")}
            autoComplete="off"
            value={name}
            maxLength={120}
            onChange={(event) => setName(event.currentTarget.value)}
            onFocus={(event) => event.currentTarget.select()}
          />
        </DialogField>
      </form>
    </Dialog>
  );
}
