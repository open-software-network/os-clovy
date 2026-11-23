import { IconCheckmark2 } from "central-icons-filled/IconCheckmark2";
import { IconFolder1 } from "central-icons/IconFolder1";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { formatDate, useT } from "../../i18n";
import { messageFromError } from "../../lib/errors";
import { useScrollFade } from "../../lib/use-scroll-fade";
import {
  discoverClaudeProjects,
  importClaudeProjects,
  type ClaudeProjectCandidate,
  type FolderDto,
} from "../../lib/tauri";
import { Dialog } from "../ui/Dialog";
import { RuntimeLogo } from "../brand/RuntimeLogo";

export function ImportClaudeProjectsDialog({
  open,
  onClose,
  onImported,
}: {
  open: boolean;
  onClose: () => void;
  onImported: (folders: FolderDto[]) => void;
}) {
  const t = useT();
  const [candidates, setCandidates] = useState<ClaudeProjectCandidate[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string>();
  const [importing, setImporting] = useState(false);
  const listRef = useRef<HTMLUListElement>(null);
  const discoveryRequestRef = useRef(0);
  const listFade = useScrollFade(listRef);

  const loadCandidates = useCallback(() => {
    const request = ++discoveryRequestRef.current;
    setCandidates(null);
    setSelected(new Set());
    setError(undefined);
    discoverClaudeProjects()
      .then((items) => {
        if (request !== discoveryRequestRef.current) return;
        setCandidates(items);
        setSelected(new Set(items.filter((item) => !item.alreadyAdded).map((item) => item.path)));
      })
      .catch((caught: unknown) => {
        if (request === discoveryRequestRef.current) setError(messageFromError(caught));
      });
  }, []);

  useEffect(() => {
    if (!open) {
      discoveryRequestRef.current += 1;
      return;
    }
    loadCandidates();
    return () => {
      discoveryRequestRef.current += 1;
    };
  }, [loadCandidates, open]);

  const available = useMemo(
    () => candidates?.filter((candidate) => !candidate.alreadyAdded) ?? [],
    [candidates],
  );

  function toggle(path: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  async function submit() {
    if (selected.size === 0 || importing) return;
    setImporting(true);
    setError(undefined);
    try {
      const folders = await importClaudeProjects([...selected]);
      onImported(folders);
      onClose();
    } catch (caught) {
      setError(messageFromError(caught));
    } finally {
      setImporting(false);
    }
  }

  const count = selected.size;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("notes.import.title")}
      description={t("notes.import.description")}
      leading={<RuntimeLogo id="claude" size={16} decorative />}
      width={600}
      className="claude-projects-dialog"
      disableBackdropClose={importing}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose} disabled={importing}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="primary-action primary-solid"
            disabled={count === 0 || importing}
            onClick={() => void submit()}
          >
            {importing ? t("notes.import.adding") : t("notes.import.commit", { count })}
          </button>
        </>
      }
    >
      <div className="dialog-body claude-projects-body">
        {candidates === null && !error ? (
          <p className="claude-projects-status">{t("notes.import.looking")}</p>
        ) : null}
        {error ? (
          <div className="claude-projects-error" role="alert">
            <p>{error}</p>
            {candidates === null ? (
              <button type="button" className="btn" onClick={loadCandidates}>
                {t("common.tryAgain")}
              </button>
            ) : null}
          </div>
        ) : null}
        {candidates && candidates.length === 0 ? (
          <div className="claude-projects-empty">
            <IconFolder1 size={20} />
            <div>
              <strong>{t("notes.import.noneTitle")}</strong>
              <p>{t("notes.import.noneBody")}</p>
            </div>
          </div>
        ) : null}
        {candidates && candidates.length > 0 && available.length === 0 ? (
          <div className="claude-projects-empty">
            <IconCheckmark2 size={20} />
            <div>
              <strong>{t("notes.import.allTitle")}</strong>
              <p>{t("notes.import.allBody")}</p>
            </div>
          </div>
        ) : null}
        {available.length > 0 ? (
          <>
            <div className="claude-projects-selection-bar">
              <span>{t("notes.import.found", { count: available.length })}</span>
              <button
                type="button"
                onClick={() =>
                  setSelected(
                    selected.size === available.length
                      ? new Set()
                      : new Set(available.map((item) => item.path)),
                  )
                }
              >
                {selected.size === available.length
                  ? t("notes.list.clearSelection")
                  : t("notes.list.selectAll")}
              </button>
            </div>
            <ul ref={listRef} className="claude-projects-list scroll-fade-mask" {...listFade.props}>
              {available.map((candidate) => (
                <li key={candidate.path}>
                  <label className="claude-project-row">
                    <input
                      type="checkbox"
                      checked={selected.has(candidate.path)}
                      onChange={() => toggle(candidate.path)}
                    />
                    <span className="claude-project-row-icon" aria-hidden>
                      <IconFolder1 size={14} />
                    </span>
                    <span className="claude-project-row-copy">
                      <strong>{candidate.name}</strong>
                      <span title={candidate.path}>{candidate.path}</span>
                    </span>
                    {candidate.lastUsedAt ? (
                      <time dateTime={candidate.lastUsedAt}>
                        {formatLastUsed(candidate.lastUsedAt)}
                      </time>
                    ) : null}
                  </label>
                </li>
              ))}
            </ul>
          </>
        ) : null}
      </div>
    </Dialog>
  );
}

function formatLastUsed(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return formatDate(date, {
    month: "short",
    day: "numeric",
  });
}
