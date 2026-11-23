import { type FormEvent, useEffect, useState } from "react";
import { t as translate, useT } from "../../i18n";
import { useClovyDataChanged } from "../../lib/clovy-data-events";
import {
  listMemories,
  memorySettings,
  setFolderInstructions,
  setFolderMemoryDisabled,
  type FolderDto,
} from "../../lib/tauri";
import { Dialog, DialogField } from "../ui/Dialog";
import { Switch } from "../ui/Switch";

const INSTRUCTIONS_MAX_CHARS = 4_000;
const INSTRUCTIONS_COUNT_FROM = 3_600;

type ProjectSettingsDialogProps = {
  open: boolean;
  folder: FolderDto;
  onClose: () => void;
  /** Persist name / description (the existing rename path). */
  onSaveDetails: (name: string, description?: string) => Promise<unknown> | void;
  /** Reflect a server-updated folder (instructions, memory toggle) into app state. */
  onFolderUpdated: (folder: FolderDto) => void;
  /** Open the full Memory manager filtered to this project. */
  onManageMemory: (folderId: string) => void;
  /** Hand delete back to the caller so it can run its own confirmation. */
  onRequestDelete: () => void;
};

/**
 * Everything about a project that's *bounded*: its name, description, the
 * instructions Clovy follows in its sessions, whether it keeps memory, and the
 * delete action. Memory itself is an unbounded, growing dataset, so it doesn't
 * live inline here — the section is a toggle plus a count that links out to the
 * full Memory manager (Settings > Memory), which is built to scale. Name /
 * description / instructions commit on Save; the memory toggle applies
 * immediately (it's a setting, not part of the form).
 */
export function ProjectSettingsDialog({
  open,
  folder,
  onClose,
  onSaveDetails,
  onFolderUpdated,
  onManageMemory,
  onRequestDelete,
}: ProjectSettingsDialogProps) {
  const t = useT();
  const [name, setName] = useState(folder.name);
  const [description, setDescription] = useState(folder.description ?? "");
  const [instructions, setInstructions] = useState(folder.instructions ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();

  const [memoryCount, setMemoryCount] = useState<number>();
  const [memoryEnabled, setMemoryEnabled] = useState(false);
  const [memoryError, setMemoryError] = useState<string>();
  useClovyDataChanged("memories", async (_change, isCurrent) => {
    if (!open) return;
    try {
      const [nextMemories, settings] = await Promise.all([
        listMemories(folder.id, false),
        memorySettings(),
      ]);
      if (!isCurrent()) return;
      setMemoryCount(nextMemories.length);
      setMemoryEnabled(settings.enabled);
      setMemoryError(undefined);
    } catch (caught) {
      if (isCurrent()) setMemoryError(messageFromCaught(caught));
    }
  });

  useEffect(() => {
    if (!open) return;
    setName(folder.name);
    setDescription(folder.description ?? "");
    setInstructions(folder.instructions ?? "");
    setSaving(false);
    setError(undefined);
  }, [open, folder.name, folder.description, folder.instructions]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setMemoryCount(undefined);
    setMemoryError(undefined);
    void Promise.all([listMemories(folder.id, false), memorySettings()])
      .then(([nextMemories, settings]) => {
        if (!active) return;
        setMemoryCount(nextMemories.length);
        setMemoryEnabled(settings.enabled);
      })
      .catch((caught) => {
        if (active) setMemoryError(messageFromCaught(caught));
      });
    return () => {
      active = false;
    };
  }, [open, folder.id]);

  function handleClose() {
    if (saving) return;
    onClose();
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedName = name.trim();
    if (!trimmedName || saving) return;
    if (instructions.length > INSTRUCTIONS_MAX_CHARS) {
      setError(
        t("notes.projectSettings.instructionsTooLong", { max: String(INSTRUCTIONS_MAX_CHARS) }),
      );
      return;
    }
    setSaving(true);
    try {
      const trimmedDescription = description.trim();
      const detailsChanged =
        trimmedName !== folder.name || trimmedDescription !== (folder.description ?? "");
      if (detailsChanged) {
        await onSaveDetails(trimmedName, trimmedDescription || undefined);
      }

      const trimmedInstructions = instructions.trim();
      if (trimmedInstructions !== (folder.instructions ?? "")) {
        const updated = await setFolderInstructions(folder.id, trimmedInstructions || undefined);
        onFolderUpdated(updated);
      }
      onClose();
    } catch (caught) {
      setError(messageFromCaught(caught));
    } finally {
      setSaving(false);
    }
  }

  async function toggleProjectMemory(remember: boolean) {
    try {
      const updated = await setFolderMemoryDisabled(folder.id, !remember);
      onFolderUpdated(updated);
      setMemoryError(undefined);
    } catch (caught) {
      setMemoryError(messageFromCaught(caught));
    }
  }

  const overLimit = instructions.length > INSTRUCTIONS_MAX_CHARS;

  return (
    <Dialog
      open={open}
      onClose={handleClose}
      title={t("notes.projectSettings.title")}
      width={520}
      className="project-settings-dialog"
      initialFocusSelector='input[name="project-name"]'
      footer={
        <>
          <button
            type="button"
            className="primary-action primary-destructive-ghost"
            disabled={saving}
            onClick={() => {
              onClose();
              onRequestDelete();
            }}
          >
            {t("notes.projectSettings.delete")}
          </button>
          <div className="dialog-footer-group">
            <button
              type="button"
              className="primary-action"
              onClick={handleClose}
              disabled={saving}
            >
              {t("common.cancel")}
            </button>
            <button
              type="submit"
              form="project-settings-form"
              className="primary-action primary-solid"
              disabled={saving || name.trim().length === 0 || overLimit}
            >
              {saving ? t("notes.folders.edit.saving") : t("common.save")}
            </button>
          </div>
        </>
      }
    >
      <form id="project-settings-form" className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("notes.folders.field.name")} htmlFor="project-name">
          <input
            id="project-name"
            name="project-name"
            className="dialog-input"
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
            maxLength={120}
          />
        </DialogField>

        <DialogField label={t("notes.folders.field.description")} htmlFor="project-description">
          <textarea
            id="project-description"
            name="project-description"
            className="dialog-textarea"
            placeholder={t("notes.folders.field.descriptionPlaceholder")}
            value={description}
            onChange={(event) => setDescription(event.currentTarget.value)}
            rows={2}
            maxLength={400}
          />
        </DialogField>

        <DialogField
          label={t("notes.projectSettings.instructions")}
          htmlFor="project-instructions"
          hint={t("notes.projectSettings.instructionsHint")}
        >
          <textarea
            id="project-instructions"
            name="project-instructions"
            className="dialog-textarea"
            placeholder={t("notes.projectSettings.instructionsPlaceholder")}
            value={instructions}
            aria-label={t("notes.projectSettings.instructionsLabel")}
            aria-invalid={overLimit || undefined}
            rows={4}
            onChange={(event) => {
              setInstructions(event.currentTarget.value);
              setError(undefined);
            }}
          />
          {instructions.length >= INSTRUCTIONS_COUNT_FROM ? (
            <p className="dialog-field-count" data-over-limit={overLimit || undefined}>
              {t("notes.projectSettings.charCount", {
                count: String(instructions.length),
                max: String(INSTRUCTIONS_MAX_CHARS),
              })}
            </p>
          ) : null}
        </DialogField>

        {error ? (
          <p className="settings-row-error" role="alert">
            {error}
          </p>
        ) : null}

        <section className="project-settings-memory" aria-labelledby="project-memory-heading">
          <div className="settings-row project-settings-memory-toggle">
            <div className="settings-row-info">
              <h3 id="project-memory-heading" className="settings-row-title">
                {t("notes.projectSettings.memory")}
              </h3>
              <p className="settings-row-description">
                {memoryEnabled
                  ? t("notes.projectSettings.memoryOn")
                  : t("notes.projectSettings.memoryOff")}
              </p>
            </div>
            <div className="settings-row-control">
              <Switch
                checked={!folder.memoryDisabled}
                disabled={!memoryEnabled}
                aria-label={t("notes.projectSettings.rememberLabel")}
                onCheckedChange={(remember) => void toggleProjectMemory(remember)}
              />
            </div>
          </div>
          <div className="project-settings-memory-manage">
            <span className="project-settings-memory-count">{memoryCountLabel(memoryCount)}</span>
            <button
              type="button"
              className="settings-inline-link project-settings-memory-manage-link"
              onClick={() => {
                onClose();
                onManageMemory(folder.id);
              }}
            >
              {t("notes.projectSettings.manageMemories")}
            </button>
          </div>
          {memoryError ? (
            <p className="settings-row-error" role="alert">
              {memoryError}
            </p>
          ) : null}
        </section>
      </form>
    </Dialog>
  );
}

function memoryCountLabel(count: number | undefined) {
  if (count === undefined) return translate("notes.projectSettings.loadingMemories");
  return translate("notes.projectSettings.memoryCount", { count });
}

function messageFromCaught(caught: unknown) {
  if (caught && typeof caught === "object" && "message" in caught) {
    return String((caught as { message: unknown }).message);
  }
  return String(caught);
}
