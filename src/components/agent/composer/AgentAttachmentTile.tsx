import { IconCrossSmall } from "central-icons/IconCrossSmall";

import { type TFunction, useT } from "../../../i18n";
import { FileTypeIcon } from "../FileTypeIcon";

const UPPERCASE_FILE_TYPE_LABELS = new Set([
  "pdf",
  "png",
  "jpg",
  "jpeg",
  "gif",
  "csv",
  "json",
  "html",
  "xml",
  "mp3",
  "mp4",
  "wav",
]);

function attachmentFileTypeLabel(t: TFunction, name: string): string {
  const filename = name.split(/[\\/]/).pop() ?? name;
  const extensionIndex = filename.lastIndexOf(".");
  if (extensionIndex <= 0 || extensionIndex === filename.length - 1) return t("chat.composer.file");
  const extension = filename.slice(extensionIndex + 1).toLowerCase();
  return UPPERCASE_FILE_TYPE_LABELS.has(extension) ? extension.toUpperCase() : extension;
}

export function AgentAttachmentTile({ name, onRemove }: { name: string; onRemove: () => void }) {
  const t = useT();
  return (
    <span className="agent-attachment-chip" data-kind="file" title={name}>
      <span className="agent-attachment-file-icon" aria-hidden="true">
        <FileTypeIcon name={name} size={18} />
      </span>
      <span className="agent-attachment-file-details">
        <span className="agent-attachment-name">{name}</span>
        <span className="agent-attachment-file-meta">
          <span className="agent-attachment-file-type">{attachmentFileTypeLabel(t, name)}</span>
        </span>
      </span>
      <button
        type="button"
        aria-label={t("chat.composer.removeNamed", { name })}
        onClick={onRemove}
      >
        <IconCrossSmall size={12} aria-hidden />
      </button>
    </span>
  );
}
