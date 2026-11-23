import { useEffect, useState } from "react";
import type { FolderDto } from "../../lib/tauri";
import { useT } from "../../i18n";
import { Dialog, DialogField } from "../ui/Dialog";

type EditFolderDialogProps = {
  open: boolean;
  onClose: () => void;
  folder: FolderDto;
  onSave: (name: string, description?: string) => Promise<unknown> | void;
};

export function EditFolderDialog({ open, onClose, folder, onSave }: EditFolderDialogProps) {
  const t = useT();
  const [name, setName] = useState(folder.name);
  const [description, setDescription] = useState(folder.description ?? "");
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(folder.name);
    setDescription(folder.description ?? "");
    setSubmitting(false);
  }, [open, folder.name, folder.description]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || submitting) return;
    setSubmitting(true);
    try {
      await onSave(trimmed, description.trim() ? description.trim() : undefined);
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
      title={t("notes.folders.edit.title")}
      description={t("notes.folders.edit.description")}
      initialFocusSelector='input[name="edit-folder-name"]'
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose} disabled={submitting}>
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            form="edit-folder-form"
            className="primary-action primary-solid"
            disabled={submitting || name.trim().length === 0}
          >
            {submitting ? t("notes.folders.edit.saving") : t("common.save")}
          </button>
        </>
      }
    >
      <form id="edit-folder-form" className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("notes.folders.field.name")} htmlFor="edit-folder-name">
          <input
            id="edit-folder-name"
            name="edit-folder-name"
            className="dialog-input"
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            maxLength={120}
          />
        </DialogField>
        <DialogField label={t("notes.folders.field.description")} htmlFor="edit-folder-description">
          <textarea
            id="edit-folder-description"
            name="edit-folder-description"
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
