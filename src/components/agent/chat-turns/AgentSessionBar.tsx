import { IconDotGrid1x3Horizontal } from "central-icons/IconDotGrid1x3Horizontal";
import { IconFiles } from "central-icons/IconFiles";
import { IconFolderAddRight } from "central-icons/IconFolderAddRight";
import { IconMoveFolder } from "central-icons/IconMoveFolder";
import { IconPencil } from "central-icons/IconPencil";
import { IconTrashCan } from "central-icons/IconTrashCan";
import { IconAnalytics } from "central-icons/IconAnalytics";
import { useEffect, useRef, useState } from "react";
import { useT } from "../../../i18n";
import { BackButton } from "../../ui/BackButton";
import { Dialog } from "../../ui/Dialog";
import type { AgentProjectContext } from "../../../lib/agent-project-context";
import type { ModelPrivacyBadge } from "../../../lib/model-privacy";
import type { AgentWorkspaceOrigin } from "../agent-workspace-types";
import { PrivacyModeBadge, UnrestrictedBadge } from "./SessionBadges";

// Persistent, full-width session bar — same chrome as the Notes/Folders
// breadcrumb. Stays pinned while the conversation scrolls beneath it, carries
// the back arrow + origin crumbs (Projects / {project} or Agents), the
// private-mode badge, and folds rename/delete into an overflow menu so the
// conversation keeps the focus (no separate title heading).
export function AgentSessionBar({
  origin,
  privacyBadge,
  fullMode,
  title,
  artifactCount = 0,
  artifactsOpen = false,
  inProject = false,
  projectContext,
  onToggleArtifacts,
  onRename,
  onUsage,
  onMoveToProject,
  onDelete,
}: {
  origin?: AgentWorkspaceOrigin;
  privacyBadge?: ModelPrivacyBadge;
  fullMode?: boolean;
  title?: string;
  artifactCount?: number;
  artifactsOpen?: boolean;
  inProject?: boolean;
  projectContext?: AgentProjectContext;
  onToggleArtifacts?: () => void;
  onRename?: (title: string) => void;
  onUsage?: () => void;
  /** Opens the change-project dialog (which also owns removal). */
  onMoveToProject?: () => void;
  onDelete?: () => void;
}) {
  const t = useT();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(title ?? "");
  const [menuOpen, setMenuOpen] = useState(false);
  const [instructionsOpen, setInstructionsOpen] = useState(false);
  const menuWrapRef = useRef<HTMLDivElement>(null);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!renaming) return;
    renameInputRef.current?.focus();
    renameInputRef.current?.select();
  }, [renaming]);

  useEffect(() => {
    if (!menuOpen) return;
    function onPointer(event: MouseEvent) {
      if (!menuWrapRef.current?.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setMenuOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  function commitRename() {
    setRenaming(false);
    onRename?.(draft);
  }

  const hasMenu = Boolean(onRename || onUsage || onMoveToProject || onDelete);

  return (
    <div className="detail-bar agent-session-bar" data-tauri-drag-region>
      {origin ? <BackButton label={origin.backLabel} onClick={origin.onBack} /> : null}
      <nav className="detail-breadcrumb" aria-label={t("chat.session.breadcrumb")}>
        <ol>
          {origin ? (
            origin.crumbs.map((crumb, index) => (
              <li key={`${crumb.label}-${index}`}>
                {index > 0 ? (
                  <span className="detail-breadcrumb-separator" aria-hidden>
                    /
                  </span>
                ) : null}
                <button type="button" className="detail-breadcrumb-link" onClick={crumb.onClick}>
                  {crumb.icon ? (
                    <span className="detail-breadcrumb-icon" aria-hidden>
                      {crumb.icon}
                    </span>
                  ) : null}
                  {crumb.label}
                </button>
              </li>
            ))
          ) : (
            <li>
              <span className="detail-breadcrumb-label">{t("chat.session.session")}</span>
            </li>
          )}
          {title !== undefined ? (
            <li>
              <span className="detail-breadcrumb-separator" aria-hidden>
                /
              </span>
              {renaming ? (
                <input
                  ref={renameInputRef}
                  className="agent-session-rename"
                  aria-label={t("chat.session.name")}
                  value={draft}
                  onChange={(event) => setDraft(event.currentTarget.value)}
                  onBlur={commitRename}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      commitRename();
                    }
                    if (event.key === "Escape") {
                      setRenaming(false);
                      setDraft(title ?? "");
                    }
                  }}
                />
              ) : (
                <span className="detail-breadcrumb-current-group">
                  <span className="detail-breadcrumb-current">
                    {title || t("chat.session.untitled")}
                  </span>
                </span>
              )}
            </li>
          ) : origin ? (
            <li>
              <span className="detail-breadcrumb-separator" aria-hidden>
                /
              </span>
              <span className="detail-breadcrumb-current">{t("chat.session.new")}</span>
            </li>
          ) : null}
        </ol>
      </nav>
      <div className="detail-bar-actions">
        {projectContext ? (
          <button
            type="button"
            className="agent-project-instructions"
            onClick={() => setInstructionsOpen(true)}
          >
            {t("chat.session.projectInstructions")}
          </button>
        ) : null}
        {fullMode ? <UnrestrictedBadge /> : null}
        {onToggleArtifacts && artifactCount > 0 ? (
          <button
            type="button"
            className="agent-session-files"
            aria-label={t("chat.session.viewFilesCount", { count: artifactCount })}
            title={t("chat.session.viewFiles")}
            aria-pressed={artifactsOpen}
            onClick={onToggleArtifacts}
          >
            <IconFiles size={14} />
            <span aria-hidden>{artifactCount}</span>
          </button>
        ) : null}
        <PrivacyModeBadge badge={privacyBadge} />
        {hasMenu ? (
          <div className="agent-session-menu-wrap" ref={menuWrapRef}>
            <button
              ref={menuTriggerRef}
              type="button"
              className="icon-button agent-session-menu-trigger"
              aria-label={t("chat.session.actions")}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => setMenuOpen((open) => !open)}
            >
              <IconDotGrid1x3Horizontal size={16} />
            </button>
            {menuOpen ? (
              <div className="sidebar-identity-menu agent-session-menu" role="menu">
                {onRename ? (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenuOpen(false);
                      setDraft(title ?? "");
                      setRenaming(true);
                    }}
                  >
                    <IconPencil size={14} />
                    {t("common.rename")}
                  </button>
                ) : null}
                {onUsage ? (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      menuTriggerRef.current?.focus();
                      setMenuOpen(false);
                      onUsage();
                    }}
                  >
                    <IconAnalytics size={14} />
                    {t("chat.session.usage")}
                  </button>
                ) : null}
                {onMoveToProject ? (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setMenuOpen(false);
                      onMoveToProject();
                    }}
                  >
                    {inProject ? <IconMoveFolder size={14} /> : <IconFolderAddRight size={14} />}
                    {inProject ? t("chat.session.changeProject") : t("chat.session.addToProject")}
                  </button>
                ) : null}
                {onDelete && (onRename || onMoveToProject) ? (
                  <div className="context-menu-separator" role="separator" />
                ) : null}
                {onDelete ? (
                  <button
                    type="button"
                    role="menuitem"
                    className="destructive"
                    onClick={() => {
                      setMenuOpen(false);
                      onDelete();
                    }}
                  >
                    <IconTrashCan size={14} />
                    {t("chat.session.delete")}
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
      <Dialog
        open={instructionsOpen}
        onClose={() => setInstructionsOpen(false)}
        title={
          projectContext?.name
            ? t("chat.session.namedProjectInstructions", { name: projectContext.name })
            : t("chat.session.projectInstructions")
        }
        footer={
          <button
            type="button"
            className="primary-action"
            onClick={() => setInstructionsOpen(false)}
          >
            {t("common.close")}
          </button>
        }
      >
        <div className="agent-project-instructions-content">
          {projectContext?.instructions?.trim() || t("chat.session.noProjectInstructions")}
        </div>
      </Dialog>
    </div>
  );
}
