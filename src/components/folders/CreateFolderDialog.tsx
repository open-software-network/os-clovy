import { useEffect, useState } from "react";
import { useT } from "../../i18n";
import { Dialog, DialogField } from "../ui/Dialog";

type CreateFolderDialogProps = {
  open: boolean;
  onClose: () => void;
  /** Optional seed name (e.g. when "Create new" is triggered from a search). */
  defaultName?: string;
  onCreate: (name: string, description?: string) => Promise<unknown> | void;
};

export function CreateFolderDialog({
  open,
  onClose,
  defaultName,
  onCreate,
}: CreateFolderDialogProps) {
  const t = useT();
  const [name, setName] = useState(defaultName ?? "");
  const [description, setDescription] = useState("");
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(defaultName ?? "");
    setDescription("");
    setSubmitting(false);
  }, [open, defaultName]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || submitting) return;
    setSubmitting(true);
    try {
      await onCreate(trimmed, description.trim() ? description.trim() : undefined);
      onClose();
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={() => {
        if (submitting) return;
        onClose();
      }}
      title={t("notes.folders.create.title")}
      description={t("notes.folders.create.description")}
      initialFocusSelector='input[name="folder-name"]'
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose} disabled={submitting}>
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            form="create-folder-form"
            className="primary-action primary-solid"
            disabled={submitting || name.trim().length === 0}
          >
            {submitting ? t("notes.folders.create.creating") : t("notes.folders.create.title")}
          </button>
        </>
      }
    >
      <form id="create-folder-form" className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("notes.folders.field.name")} htmlFor="folder-name">
          <input
            id="folder-name"
            name="folder-name"
            className="dialog-input"
            placeholder={t("notes.folders.field.namePlaceholder")}
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            maxLength={120}
          />
        </DialogField>
        <DialogField label={t("notes.folders.field.description")} htmlFor="folder-description">
          <textarea
            id="folder-description"
            name="folder-description"
            className="dialog-textarea"
            placeholder={t("notes.folders.field.descriptionPlaceholder")}
            value={description}
            onChange={(event) => setDescription(event.currentTarget.value)}
            rows={3}
            maxLength={400}
          />
        </DialogField>
      </form>
    </Dialog>
  );
}
