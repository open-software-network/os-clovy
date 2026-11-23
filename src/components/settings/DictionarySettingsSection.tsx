import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconPencilLine } from "central-icons/IconPencilLine";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconTrashCanSimple } from "central-icons/IconTrashCanSimple";
import type { FormEvent } from "react";
import { useEffect, useMemo, useState } from "react";
import {
  createDictionaryEntry,
  deleteDictionaryEntry,
  listDictionaryEntries,
  updateDictionaryEntry,
} from "../../lib/tauri";
import type { DictionaryEntryDto } from "../../lib/tauri";
import { Dialog, DialogField } from "../ui/Dialog";
import { useT } from "../../i18n";

type Draft = {
  phrase: string;
};

const EMPTY_DRAFT: Draft = {
  phrase: "",
};

export function DictionarySettingsSection() {
  const t = useT();
  const [entries, setEntries] = useState<DictionaryEntryDto[]>([]);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [editingId, setEditingId] = useState<string>();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string>();
  const [saveError, setSaveError] = useState<string>();

  useEffect(() => {
    void refresh();
  }, []);

  const visibleEntries = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return entries;
    return entries.filter((entry) => entry.phrase.toLowerCase().includes(normalized));
  }, [entries, query]);

  async function refresh() {
    try {
      setEntries(await listDictionaryEntries());
    } catch (caught) {
      setError(messageFromError(caught));
    }
  }

  async function saveEntry() {
    const phrase = draft.phrase.trim();
    if (!phrase) return;
    try {
      const entry = editingId
        ? await updateDictionaryEntry({ entryId: editingId, phrase })
        : await createDictionaryEntry({ phrase });
      setEntries((current) =>
        editingId
          ? current.map((item) => (item.id === entry.id ? entry : item))
          : [...current, entry].sort(compareEntries),
      );
      closeDialog();
    } catch (caught) {
      setSaveError(messageFromError(caught));
    }
  }

  async function removeEntry(entryId: string) {
    try {
      await deleteDictionaryEntry(entryId);
      setEntries((current) => current.filter((entry) => entry.id !== entryId));
      setError(undefined);
    } catch (caught) {
      setError(messageFromError(caught));
    }
  }

  function startCreating() {
    setEditingId(undefined);
    setDraft(EMPTY_DRAFT);
    setSaveError(undefined);
    setDialogOpen(true);
  }

  function startEditing(entry: DictionaryEntryDto) {
    setEditingId(entry.id);
    setDraft({ phrase: entry.phrase });
    setSaveError(undefined);
    setDialogOpen(true);
  }

  function closeDialog() {
    setDialogOpen(false);
    setDraft(EMPTY_DRAFT);
    setEditingId(undefined);
    setSaveError(undefined);
  }

  const emptyMessage =
    entries.length === 0
      ? t("settings.dictionary.empty")
      : t("settings.dictionary.noMatch", { query: query.trim() });

  return (
    <section className="settings-group" aria-labelledby="dictionary-heading">
      <h2 id="dictionary-heading" className="settings-group-heading">
        {t("settings.dictionary.title")}
      </h2>
      <p className="settings-group-description">{t("settings.dictionary.description")}</p>
      <div className="settings-card dictionary-card">
        <div className="dictionary-toolbar">
          <label className="folders-search">
            <IconMagnifyingGlass size={14} />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder={t("common.search")}
              aria-label={t("settings.dictionary.searchAria")}
            />
          </label>
          <button type="button" className="primary-action primary-solid" onClick={startCreating}>
            <IconPlusMedium size={14} />
            {t("settings.dictionary.add")}
          </button>
        </div>
        {visibleEntries.length === 0 ? (
          <p className="settings-empty">{emptyMessage}</p>
        ) : (
          <div className="settings-rows">
            {visibleEntries.map((entry) => (
              <div key={entry.id} className="settings-row settings-row-compact">
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{entry.phrase}</h3>
                </div>
                <div className="settings-row-control">
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t("settings.dictionary.editAria", { phrase: entry.phrase })}
                    onClick={() => startEditing(entry)}
                  >
                    <IconPencilLine size={14} />
                  </button>
                  <button
                    type="button"
                    className="icon-button icon-button-destructive"
                    aria-label={t("settings.dictionary.deleteAria", { phrase: entry.phrase })}
                    onClick={() => void removeEntry(entry.id)}
                  >
                    <IconTrashCanSimple size={14} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
      {error ? <p className="settings-row-error">{error}</p> : null}

      <DictionaryEntryDialog
        open={dialogOpen}
        phrase={draft.phrase}
        editing={editingId !== undefined}
        error={saveError}
        onChange={(phrase) => {
          setDraft({ phrase });
          if (saveError) setSaveError(undefined);
        }}
        onClose={closeDialog}
        onSave={() => void saveEntry()}
      />
    </section>
  );
}

function DictionaryEntryDialog({
  open,
  phrase,
  editing,
  error,
  onChange,
  onClose,
  onSave,
}: {
  open: boolean;
  phrase: string;
  editing: boolean;
  error?: string;
  onChange: (phrase: string) => void;
  onClose: () => void;
  onSave: () => void;
}) {
  const t = useT();
  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (phrase.trim()) onSave();
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={editing ? t("settings.dictionary.editTitle") : t("settings.dictionary.addTitle")}
      initialFocusSelector='input[name="dictionary-phrase"]'
      footer={
        <>
          <button type="button" className="primary-action" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            form="dictionary-entry-form"
            className="primary-action primary-solid"
            disabled={phrase.trim().length === 0}
          >
            {editing ? t("settings.dictionary.saveChanges") : t("settings.dictionary.add")}
          </button>
        </>
      }
    >
      <form id="dictionary-entry-form" className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("settings.dictionary.field")} htmlFor="dictionary-phrase">
          <input
            id="dictionary-phrase"
            name="dictionary-phrase"
            className="dialog-input"
            value={phrase}
            onChange={(event) => onChange(event.currentTarget.value)}
            placeholder={t("settings.dictionary.placeholder")}
            autoComplete="off"
            maxLength={160}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? "dictionary-phrase-error" : undefined}
          />
        </DialogField>
        {error ? (
          <p id="dictionary-phrase-error" className="settings-row-error">
            {error}
          </p>
        ) : null}
      </form>
    </Dialog>
  );
}

function compareEntries(left: DictionaryEntryDto, right: DictionaryEntryDto) {
  return left.phrase.localeCompare(right.phrase, undefined, {
    sensitivity: "base",
  });
}

function messageFromError(caught: unknown) {
  if (caught && typeof caught === "object" && "message" in caught) {
    return String((caught as { message: unknown }).message);
  }
  return String(caught);
}
