import { IconBrain } from "central-icons/IconBrain";
import { IconChevronRightSmall } from "central-icons/IconChevronRightSmall";
import { IconMagnifyingGlass } from "central-icons/IconMagnifyingGlass";
import { IconPencilLine } from "central-icons/IconPencilLine";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconProjects } from "central-icons/IconProjects";
import { IconTrashCanSimple } from "central-icons/IconTrashCanSimple";
import type { FormEvent } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { formatDate, useT } from "../../i18n";
import { getCurrentDataPartitionName } from "../../lib/data-partition";
import { useClovyDataChanged } from "../../lib/clovy-data-events";
import { useProjectMemoryDemo } from "../../lib/project-memory-demo";
import {
  createMemory,
  deleteMemory,
  listMemories,
  memorySettings,
  setMemoryEnabled,
  updateMemory,
  type FolderDto,
  type MemoryDto,
} from "../../lib/tauri";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Dialog, DialogField } from "../ui/Dialog";
import { EmptyState } from "../ui/EmptyState";
import { Select } from "../ui/Select";
import { Switch } from "../ui/Switch";
import { SettingsPageHeader } from "./AppSettings";

const MEMORY_MAX_CHARS = 4_000;
// Filter sentinel for memories that aren't tied to any project.
const SCOPE_ALL = "__all__";
const SCOPE_GENERAL = "__general__";

export function MemorySettingsSection({
  folders,
  initialFolderFilter,
  onOpenProject,
}: {
  folders: FolderDto[];
  /** When deep-linked from a project, pre-filter the manager to it. */
  initialFolderFilter?: string;
  /** Drill from a memory's project tag into that project. */
  onOpenProject?: (folderId: string) => void;
}) {
  const t = useT();
  const [memories, setMemories] = useState<MemoryDto[]>([]);
  const [enabled, setEnabled] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [savingEnabled, setSavingEnabled] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [error, setError] = useState<string>();
  const [mutationError, setMutationError] = useState<string>();
  const [memoryDialogOpen, setMemoryDialogOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<string>(initialFolderFilter ?? SCOPE_ALL);
  const refreshVersion = useRef(0);
  const mounted = useRef(true);
  const refresh = useCallback(
    async (eventIsCurrent: () => boolean = () => true, reconcileSetting = false) => {
      const version = ++refreshVersion.current;
      const profile = getCurrentDataPartitionName();
      const isCurrent = () =>
        mounted.current &&
        version === refreshVersion.current &&
        profile === getCurrentDataPartitionName() &&
        eventIsCurrent();
      try {
        // Every memory — global and per-project — so this page is the one place
        // to browse and prune all of them.
        const settingsRead = memorySettings();
        if (reconcileSetting) {
          // A failed runtime toggle may already have persisted. Publish that
          // setting even if the separate list read waits or fails.
          void settingsRead.then(
            (settings) => {
              if (isCurrent()) setEnabled(settings.enabled);
            },
            () => undefined,
          );
        }
        const [nextMemories, settings] = await Promise.all([
          listMemories(undefined, true),
          settingsRead,
        ]);
        if (!isCurrent()) return;
        setMemories(sortNewestFirst(nextMemories));
        setEnabled(settings.enabled);
        setError(undefined);
      } catch (caught) {
        if (isCurrent()) setError(messageFromError(caught));
      } finally {
        if (isCurrent()) setLoaded(true);
      }
    },
    [],
  );
  useClovyDataChanged("memories", (_change, isCurrent) => refresh(isCurrent));

  // __projectMemoryDemo() (dev console): populate the manager with sample
  // memories, spread across real projects, to design the list at scale.
  const demo = useProjectMemoryDemo();
  const allMemories = useMemo(
    () => (demo ? decorateDemoMemories(demo, folders) : memories),
    [demo, folders, memories],
  );

  useEffect(() => {
    mounted.current = true;
    void refresh();
    return () => {
      mounted.current = false;
      refreshVersion.current += 1;
    };
  }, [refresh]);

  // Follow the deep-link: opening Memory from a project scopes to it; opening
  // it from the settings nav (filter cleared upstream) resets to all.
  useEffect(() => {
    setScope(initialFolderFilter ?? SCOPE_ALL);
  }, [initialFolderFilter]);

  function beginMutation() {
    refreshVersion.current += 1;
    const profile = getCurrentDataPartitionName();
    return () => mounted.current && profile === getCurrentDataPartitionName();
  }

  function applyMutation(isCurrent: () => boolean, apply: () => void) {
    if (!isCurrent()) return;
    // Invalidate reads that began both before and during persistence.
    refreshVersion.current += 1;
    apply();
    setMutationError(undefined);
    // One authoritative reload after the accepted transition also retains
    // unrelated native changes whose earlier snapshots were invalidated.
    void refresh();
  }

  async function toggleEnabled(next: boolean) {
    if (savingEnabled) return;
    setSavingEnabled(true);
    const isCurrent = beginMutation();
    try {
      const settings = await setMemoryEnabled(next);
      applyMutation(isCurrent, () => {
        setEnabled(settings.enabled);
        setError(undefined);
      });
    } catch {
      // The setting may already be persisted even if runtime enforcement failed.
      // Reconcile the whole visible store without losing the mutation error.
      if (isCurrent()) {
        setMutationError(t("settingsPanels.memory.toggleError"));
        void refresh(isCurrent, true);
      }
    } finally {
      if (isCurrent()) setSavingEnabled(false);
    }
  }

  function reconcileFailedMutation(isCurrent: () => boolean, caught: unknown) {
    if (!isCurrent()) return;
    setMutationError(messageFromError(caught));
    // Failure also invalidated the pending native snapshot. Read authoritative
    // state again; read errors cannot replace the useful mutation failure.
    void refresh();
  }

  async function addMemory(content: string, folderId?: string) {
    const isCurrent = beginMutation();
    try {
      const created = await createMemory({ content, folderId, source: "user" });
      applyMutation(isCurrent, () =>
        setMemories((current) => sortNewestFirst([created, ...current])),
      );
    } catch (caught) {
      reconcileFailedMutation(isCurrent, caught);
      throw caught;
    }
  }

  async function editMemory(id: string, content: string) {
    const isCurrent = beginMutation();
    try {
      const updated = await updateMemory(id, content);
      applyMutation(isCurrent, () =>
        setMemories((current) =>
          sortNewestFirst(current.map((memory) => (memory.id === updated.id ? updated : memory))),
        ),
      );
    } catch (caught) {
      reconcileFailedMutation(isCurrent, caught);
      throw caught;
    }
  }

  async function removeMemory(id: string) {
    const isCurrent = beginMutation();
    try {
      await deleteMemory(id);
      applyMutation(isCurrent, () => {
        setMemories((current) => current.filter((memory) => memory.id !== id));
        setError(undefined);
      });
    } catch (caught) {
      reconcileFailedMutation(isCurrent, caught);
      throw caught;
    }
  }

  const normalizedQuery = query.trim().toLowerCase();
  const filtered = useMemo(
    () =>
      allMemories.filter((memory) => {
        if (scope === SCOPE_GENERAL && memory.folderId) return false;
        if (scope !== SCOPE_ALL && scope !== SCOPE_GENERAL && memory.folderId !== scope) {
          return false;
        }
        if (normalizedQuery && !memory.content.toLowerCase().includes(normalizedQuery)) {
          return false;
        }
        return true;
      }),
    [allMemories, scope, normalizedQuery],
  );

  const scopeOptions = useMemo(
    () => [
      {
        value: SCOPE_ALL,
        label: t("settingsPanels.memory.allProjects"),
        count: allMemories.length,
      },
      {
        value: SCOPE_GENERAL,
        label: t("settingsPanels.memory.general"),
        count: allMemories.filter((memory) => !memory.folderId).length,
      },
      ...[...folders]
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
        .map((folder) => ({
          value: folder.id,
          label: folder.name,
          count: allMemories.filter((memory) => memory.folderId === folder.id).length,
        })),
    ],
    [folders, allMemories, t],
  );

  const addDefaultFolderId = scope !== SCOPE_ALL && scope !== SCOPE_GENERAL ? scope : undefined;
  const addMemoryButton = (
    <button
      type="button"
      className="primary-action primary-solid memory-add"
      disabled={!enabled || !loaded}
      onClick={() => setAddOpen(true)}
    >
      <IconPlusMedium size={14} />
      {t("settingsPanels.memory.addMemory")}
    </button>
  );

  return (
    <section className="settings-group memory-settings" aria-labelledby="memory-heading">
      <SettingsPageHeader
        id="memory-heading"
        title={t("settingsPanels.memory.title")}
        blurb={t("settingsPanels.memory.blurb")}
      />

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-info">
            <h3 className="settings-row-title">{t("settingsPanels.memory.enableTitle")}</h3>
            <p className="settings-row-description">
              {t("settingsPanels.memory.enableDescription")}
            </p>
          </div>
          <div className="settings-row-control">
            <Switch
              checked={enabled}
              disabled={!loaded || savingEnabled}
              aria-label={t("settingsPanels.memory.enableTitle")}
              onCheckedChange={(next) => void toggleEnabled(next)}
            />
          </div>
        </div>
      </div>

      {!enabled && loaded ? (
        <p className="memory-settings-hint">{t("settingsPanels.memory.offHint")}</p>
      ) : null}

      {loaded && allMemories.length === 0 && !memoryDialogOpen ? (
        <EmptyState
          className="memory-empty-state"
          label={t("settingsPanels.memory.savedMemories")}
          icon={<IconBrain size={28} />}
          title={t("settingsPanels.memory.emptyTitle")}
          description={t("settingsPanels.memory.emptyDescription")}
          action={addMemoryButton}
        />
      ) : (
        <>
          <h2 className="settings-group-heading memory-manager-heading">
            {t("settingsPanels.memory.savedMemories")}
            {loaded ? (
              <span className="status-pill memory-manager-heading-count">{allMemories.length}</span>
            ) : null}
          </h2>
          <p className="settings-group-description">
            {t("settingsPanels.memory.managerDescription")}
          </p>
          <div className="settings-card memory-manager-card">
            <div className="memory-manager-toolbar">
              <div className="settings-search memory-search">
                <IconMagnifyingGlass
                  size={15}
                  ariaHidden
                  className="settings-search-icon memory-search-icon"
                />
                <input
                  type="search"
                  aria-label={t("settingsPanels.memory.search")}
                  placeholder={t("settingsPanels.memory.search")}
                  value={query}
                  onChange={(event) => setQuery(event.currentTarget.value)}
                />
              </div>
              <Select
                className="memory-scope-select"
                value={scope}
                options={scopeOptions}
                placeholder={t("settingsPanels.memory.allProjects")}
                ariaLabel={t("settingsPanels.memory.filterByProject")}
                onChange={setScope}
                popoverWidth="trigger"
              />
              {addMemoryButton}
            </div>
            {filtered.length > 0 || memoryDialogOpen ? (
              <MemoryRows
                memories={filtered}
                folders={folders}
                editable={enabled}
                onUpdate={editMemory}
                onDelete={removeMemory}
                onOpenProject={onOpenProject}
                onDialogOpenChange={setMemoryDialogOpen}
              />
            ) : (
              <p className="memory-manager-noresults">{t("settingsPanels.memory.noResults")}</p>
            )}
          </div>
        </>
      )}
      {mutationError || error ? (
        <p className="settings-row-error" role="alert">
          {mutationError ?? error}
        </p>
      ) : null}

      <MemoryDialog
        open={addOpen}
        title={t("settingsPanels.memory.addMemory")}
        submitLabel={t("settingsPanels.memory.addMemory")}
        folders={folders}
        defaultFolderId={addDefaultFolderId}
        onClose={() => setAddOpen(false)}
        onSubmit={async (content, folderId) => {
          await addMemory(content, folderId);
          setAddOpen(false);
        }}
      />
    </section>
  );
}

export function MemoryRows({
  memories,
  folders,
  editable,
  onUpdate,
  onDelete,
  onOpenProject,
  onDialogOpenChange,
}: {
  memories: MemoryDto[];
  /** Present in the manager so each row can show its project tag. */
  folders?: FolderDto[];
  editable: boolean;
  onUpdate: (id: string, content: string) => Promise<unknown>;
  onDelete: (id: string) => Promise<unknown>;
  /** Drill from a memory's project into that project. */
  onOpenProject?: (folderId: string) => void;
  onDialogOpenChange?: (open: boolean) => void;
}) {
  const t = useT();
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());
  const [editing, setEditing] = useState<MemoryDto>();
  const [deleting, setDeleting] = useState<MemoryDto>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    onDialogOpenChange?.(editing !== undefined || deleting !== undefined);
  }, [editing, deleting, onDialogOpenChange]);

  const folderName = useMemo(() => {
    const byId = new Map((folders ?? []).map((folder) => [folder.id, folder.name]));
    return (memory: MemoryDto) => (memory.folderId ? byId.get(memory.folderId) : undefined);
  }, [folders]);

  return (
    <>
      <div className="settings-rows memory-rows">
        {memories.map((memory) => {
          const expanded = expandedIds.has(memory.id);
          const project = folderName(memory);
          return (
            <div key={memory.id} className="settings-row settings-row-compact memory-row">
              <div className="settings-row-info">
                <button
                  type="button"
                  className="memory-content"
                  data-expanded={expanded || undefined}
                  aria-expanded={expanded}
                  onClick={() =>
                    setExpandedIds((current) => {
                      const next = new Set(current);
                      if (next.has(memory.id)) next.delete(memory.id);
                      else next.add(memory.id);
                      return next;
                    })
                  }
                >
                  {memory.content}
                </button>
                <p className="memory-meta">
                  <span>
                    {memory.source === "agent"
                      ? t("settingsPanels.memory.addedByClovy")
                      : t("settingsPanels.memory.addedByYou")}
                  </span>
                  <span className="metadata-dot" aria-hidden />
                  <span>{formatMemoryDate(memory.createdAt)}</span>
                  {project && memory.folderId ? (
                    <>
                      <span className="metadata-dot" aria-hidden />
                      {onOpenProject ? (
                        <button
                          type="button"
                          className="memory-project"
                          aria-label={t("settingsPanels.memory.openProject", { name: project })}
                          onClick={() => onOpenProject(memory.folderId as string)}
                        >
                          <IconProjects size={11} />
                          {project}
                          <IconChevronRightSmall size={11} className="memory-project-chevron" />
                        </button>
                      ) : (
                        <span className="memory-project">
                          <IconProjects size={11} />
                          {project}
                        </span>
                      )}
                    </>
                  ) : null}
                </p>
              </div>
              <div className="settings-row-control">
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t("settingsPanels.memory.editMemory")}
                  disabled={!editable}
                  onClick={() => setEditing(memory)}
                >
                  <IconPencilLine size={14} />
                </button>
                <button
                  type="button"
                  className="icon-button icon-button-destructive"
                  aria-label={t("settingsPanels.memory.deleteMemory")}
                  onClick={() => setDeleting(memory)}
                >
                  <IconTrashCanSimple size={14} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {error ? (
        <p className="settings-row-error" role="alert">
          {error}
        </p>
      ) : null}

      <MemoryDialog
        open={editing !== undefined}
        title={t("settingsPanels.memory.editMemory")}
        submitLabel={t("settingsPanels.memory.saveChanges")}
        initialContent={editing?.content}
        onClose={() => {
          setEditing(undefined);
          setError(undefined);
        }}
        onSubmit={async (content) => {
          if (!editing) return;
          try {
            await onUpdate(editing.id, content);
            setEditing(undefined);
            setError(undefined);
          } catch (caught) {
            setError(messageFromError(caught));
            throw caught;
          }
        }}
      />
      <ConfirmDialog
        open={deleting !== undefined}
        title={t("settingsPanels.memory.deleteTitle")}
        description={t("settingsPanels.memory.deleteDescription")}
        confirmLabel={t("common.delete")}
        destructive
        onClose={() => setDeleting(undefined)}
        onConfirm={async () => {
          if (!deleting) return;
          await onDelete(deleting.id);
        }}
      />
    </>
  );
}

function MemoryDialog({
  open,
  title,
  submitLabel,
  initialContent = "",
  folders,
  defaultFolderId,
  onClose,
  onSubmit,
}: {
  open: boolean;
  title: string;
  submitLabel: string;
  initialContent?: string;
  folders?: FolderDto[];
  defaultFolderId?: string;
  onClose: () => void;
  onSubmit: (content: string, folderId?: string) => Promise<void>;
}) {
  const t = useT();
  const [content, setContent] = useState(initialContent);
  // null = no project (a general memory) — the optional default.
  const [scope, setScope] = useState<string | null>(defaultFolderId ?? null);
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setContent(initialContent);
    setScope(defaultFolderId ?? null);
    setError(undefined);
  }, [open, initialContent, defaultFolderId]);

  function handleClose() {
    if (saving) return;
    onClose();
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = content.trim();
    if (!trimmed || saving) return;
    setSaving(true);
    try {
      await onSubmit(trimmed, folders && scope ? scope : undefined);
    } catch (caught) {
      setError(messageFromError(caught));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={handleClose}
      title={title}
      initialFocusSelector='textarea[name="memory-content"]'
      footer={
        <>
          <button type="button" className="primary-action" disabled={saving} onClick={handleClose}>
            {t("common.cancel")}
          </button>
          <button
            type="submit"
            form="memory-entry-form"
            className="primary-action primary-solid"
            disabled={saving || content.trim().length === 0}
          >
            {submitLabel}
          </button>
        </>
      }
    >
      <form id="memory-entry-form" className="dialog-body" onSubmit={handleSubmit}>
        <DialogField label={t("settingsPanels.memory.memoryLabel")} htmlFor="memory-content">
          <textarea
            id="memory-content"
            name="memory-content"
            className="dialog-textarea"
            value={content}
            maxLength={MEMORY_MAX_CHARS}
            onChange={(event) => {
              setContent(event.currentTarget.value);
              setError(undefined);
            }}
          />
        </DialogField>
        {folders ? (
          <DialogField
            label={t("settingsPanels.memory.projectLabel")}
            hint={t("settingsPanels.memory.projectHint")}
          >
            <Select
              value={scope ?? SCOPE_GENERAL}
              options={[
                { value: SCOPE_GENERAL, label: t("settingsPanels.memory.general") },
                ...folders.map((folder) => ({ value: folder.id, label: folder.name })),
              ]}
              placeholder={t("settingsPanels.memory.selectProject")}
              ariaLabel={t("settingsPanels.memory.memoryProject")}
              onChange={(value) => setScope(value === SCOPE_GENERAL ? null : value)}
              popoverWidth="trigger"
            />
          </DialogField>
        ) : null}
        {error ? (
          <p className="settings-row-error" role="alert">
            {error}
          </p>
        ) : null}
      </form>
    </Dialog>
  );
}

/** Spread demo memories across the first real projects (leaving one General)
 * so the manager mock shows realistic project tags. */
function decorateDemoMemories(demo: MemoryDto[], folders: FolderDto[]): MemoryDto[] {
  return demo.map((memory, index) => {
    const slot = folders[index % (folders.length + 1)];
    return { ...memory, folderId: slot?.id };
  });
}

function formatMemoryDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  return formatDate(date, {
    month: "short",
    day: "numeric",
    year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
}

function sortNewestFirst(memories: MemoryDto[]) {
  return [...memories].sort((left, right) => right.createdAt.localeCompare(left.createdAt));
}

function messageFromError(caught: unknown) {
  if (caught && typeof caught === "object" && "message" in caught) {
    return String((caught as { message: unknown }).message);
  }
  return String(caught);
}
