import { t } from "../i18n/translate";
import { discardStagedAgentAttachments, stageAgentAttachmentBytes } from "./tauri";

export const MAX_AGENT_COMPOSER_ATTACHMENTS = 8;
export const MAX_AGENT_ATTACHMENT_BYTES = 50 * 1024 * 1024;

function readDroppedFileBytes(file: File): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error(t("lib.errors.fileDropUnreadable")));
    reader.onabort = () => reject(new Error(t("lib.errors.fileDropUnreadable")));
    reader.onload = () => {
      if (!(reader.result instanceof ArrayBuffer)) {
        reject(new Error(t("lib.errors.fileDropUnreadable")));
        return;
      }
      resolve(new Uint8Array(reader.result));
    };
    reader.readAsArrayBuffer(file);
  });
}

/** Stages Finder drops one at a time so only one file's bytes are resident at once. */
export async function stageDroppedAgentFiles(
  files: File[],
  existingAttachmentCount: number,
): Promise<string[]> {
  if (!files.length) {
    throw new Error(t("lib.errors.fileDropEmpty"));
  }
  if (existingAttachmentCount + files.length > MAX_AGENT_COMPOSER_ATTACHMENTS) {
    throw new Error(t("lib.errors.fileDropTooMany", { count: MAX_AGENT_COMPOSER_ATTACHMENTS }));
  }

  const stagedPaths: string[] = [];
  try {
    for (const file of files) {
      if (file.size > MAX_AGENT_ATTACHMENT_BYTES) {
        throw new Error(t("lib.errors.fileDropTooLarge"));
      }
      const bytes = await readDroppedFileBytes(file).catch(() => {
        const name = file.name || t("lib.errors.fileDropThisItem");
        throw new Error(t("lib.errors.fileDropFolder", { name }));
      });
      stagedPaths.push(await stageAgentAttachmentBytes(file.name, bytes));
    }
    return stagedPaths;
  } catch (error) {
    void discardStagedAgentAttachments(stagedPaths).catch(() => undefined);
    throw error;
  }
}
