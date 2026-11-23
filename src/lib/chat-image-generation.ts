import { t } from "../i18n/translate";
/**
 * Orchestration for generating an image from chat and handing it to Clovy's
 * EXISTING inline image display path.
 *
 * Clovy imports a pasted or dropped image into the session workspace and shows it inline as a composer
 * attachment (`attachmentStateFrom` -> `attachImageToSession`, rendered from
 * `previewDataUrl`). This module reuses that path verbatim for a GENERATED
 * image: it calls the native direct fal image command, decodes the base64 result,
 * imports the bytes into the workspace, and returns the same
 * `AgentAttachmentState` a pasted image produces. The caller drops the
 * attachment into the composer list exactly as it does for a paste.
 *
 * UI- and runtime-free: side effects are
 * injected, so the flow is unit-testable and only this seam moves if the wire
 * shape changes. It never throws.
 */

import {
  attachmentStateFrom,
  parseImageDataUrl,
  type AgentAttachmentState,
  type ImportedAgentFile,
} from "./agent-image-attachments";
import { messageFromError } from "./errors";
import type { GeneratedImageDto } from "./tauri";

export type GenerateChatImageDeps = {
  /** Calls the native direct fal image command; `model` defaults to the saved selection. */
  generate: (
    prompt: string,
    model: string | undefined,
    requestId: string,
    safeMode?: boolean,
  ) => Promise<GeneratedImageDto>;
  /** Imports raw bytes into the session workspace (the paste path's importer). */
  importImageBytes: (name: string, bytes: Uint8Array) => Promise<ImportedAgentFile>;
  /** Resolves the default image model when the caller passes none. */
  defaultModel?: () => string;
};

export type GenerateChatImageResult =
  | {
      status: "ok";
      file: ImportedAgentFile;
      /** Ready to drop into the composer attachment list, like a pasted image. */
      attachment: AgentAttachmentState;
      /** `data:<mime>;base64,<data>` for any direct inline preview. */
      dataUrl: string;
    }
  | { status: "error"; message: string };

export type EditChatImageDeps = {
  /** Reads the source workspace file as a `data:<mime>;base64,...` URL. */
  readImageData: (path: string) => Promise<string | null>;
  /** Calls the native direct fal image-edit command. */
  edit: (
    imageBase64: string,
    prompt: string,
    mimeType?: string,
    model?: string,
  ) => Promise<GeneratedImageDto>;
  /** Imports edited bytes into the session workspace. */
  importImageBytes: (name: string, bytes: Uint8Array) => Promise<ImportedAgentFile>;
};

export type EditChatImageResult = GenerateChatImageResult;

/** File extension for the workspace import, by mime. Defaults to png (what the
 * backend always requests from the provider). */
const EXTENSION_BY_MIME: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

/** Wrap a generated image in a data URL — the shape the existing display path
 * (and {@link parseImageDataUrl}) understands. */
export function generatedImageDataUrl(image: { mimeType: string; imageBase64: string }): string {
  return `data:${image.mimeType};base64,${image.imageBase64}`;
}

/** Decode a base64 string to bytes for the workspace importer. */
export function decodeBase64(base64: string): Uint8Array {
  const binary = atob(base64.trim());
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function generatedImageFileName(mimeType: string): string {
  const extension = EXTENSION_BY_MIME[mimeType.trim().toLowerCase()] ?? "png";
  // Timestamped so two generations in the same session don't collide on import.
  return `generated-image-${Date.now()}.${extension}`;
}

/**
 * Generate an image from `prompt` and return it as a composer attachment that
 * renders through Clovy's existing inline image path. Never throws: validation
 * and provider/import failures come back as `{ status: "error", message }`.
 */
export async function generateChatImage(
  prompt: string,
  deps: GenerateChatImageDeps,
  model?: string,
  requestId = newImageRequestId(),
  safeMode?: boolean,
): Promise<GenerateChatImageResult> {
  const trimmed = prompt.trim();
  if (!trimmed) {
    return { status: "error", message: t("lib.errors.imagePromptRequired") };
  }

  let image: GeneratedImageDto;
  try {
    image = await deps.generate(trimmed, model ?? deps.defaultModel?.(), requestId, safeMode);
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }

  const dataUrl = generatedImageDataUrl(image);
  // Validate through the SAME guard the paste/attach path uses, so a malformed
  // or non-image response never reaches the display surface.
  if (!parseImageDataUrl(dataUrl)) {
    return {
      status: "error",
      message: t("lib.errors.imageUndisplayable"),
    };
  }

  let file: ImportedAgentFile;
  try {
    file = await deps.importImageBytes(
      generatedImageFileName(image.mimeType),
      decodeBase64(image.imageBase64),
    );
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }

  // Same composer attachment shape a pasted/dropped image produces, so the
  // generated image renders inline through the existing display path.
  return { status: "ok", file, attachment: attachmentStateFrom(file), dataUrl };
}

export function newImageRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `image-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/**
 * Edit an existing workspace image and return the edited result through the
 * same imported-file shape as generation. Never throws.
 */
export async function editChatImage(
  source: ImportedAgentFile,
  instruction: string,
  deps: EditChatImageDeps,
  model?: string,
): Promise<EditChatImageResult> {
  const trimmed = instruction.trim();
  if (!trimmed) {
    return { status: "error", message: t("lib.errors.imageEditInstructionRequired") };
  }

  let sourceData: string | null;
  try {
    sourceData = await deps.readImageData(source.path);
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }
  const parsedSource = parseImageDataUrl(sourceData);
  if (!parsedSource) {
    return {
      status: "error",
      message: t("lib.errors.imageSourceUnreadable"),
    };
  }

  let image: GeneratedImageDto;
  try {
    image = await deps.edit(parsedSource.dataBase64, trimmed, parsedSource.mimeType, model);
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }

  const dataUrl = generatedImageDataUrl(image);
  if (!parseImageDataUrl(dataUrl)) {
    return {
      status: "error",
      message: t("lib.errors.imageUndisplayable"),
    };
  }

  let file: ImportedAgentFile;
  try {
    file = await deps.importImageBytes(
      generatedImageFileName(image.mimeType),
      decodeBase64(image.imageBase64),
    );
  } catch (error) {
    return { status: "error", message: messageFromError(error) };
  }

  return { status: "ok", file, attachment: attachmentStateFrom(file), dataUrl };
}
