import { describe, expect, it, vi } from "vitest";
import {
  decodeBase64,
  editChatImage,
  generateChatImage,
  generatedImageDataUrl,
  type EditChatImageDeps,
  type GenerateChatImageDeps,
} from "../lib/chat-image-generation";
import { parseImageDataUrl } from "../lib/agent-image-attachments";
import type { GeneratedImageDto, ImportedAgentFile } from "../lib/tauri";

// "aGVsbG8=" is base64 for "hello".
const HELLO_BASE64 = "aGVsbG8=";
const HELLO_BYTES = [104, 101, 108, 108, 111];

function pngImage(): GeneratedImageDto {
  return {
    imageBase64: HELLO_BASE64,
    mimeType: "image/png",
    model: "fal-ai/flux/dev",
    provider: "fal",
  };
}

function importedFile(): ImportedAgentFile {
  return {
    name: "generated-image-1.png",
    path: "/workspace/uploads/generated-image-1.png",
    rootLabel: "Workspace",
    size: 5,
    previewDataUrl: `data:image/png;base64,${HELLO_BASE64}`,
  };
}

describe("chat image generation", () => {
  it("decodes base64 to the original bytes", () => {
    expect(Array.from(decodeBase64(HELLO_BASE64))).toEqual(HELLO_BYTES);
  });

  it("builds a data url the existing image display path accepts", () => {
    const dataUrl = generatedImageDataUrl(pngImage());
    expect(dataUrl).toBe(`data:image/png;base64,${HELLO_BASE64}`);
    // Same guard the paste/attach path uses to decide an image is displayable.
    expect(parseImageDataUrl(dataUrl)).toEqual({
      mimeType: "image/png",
      dataBase64: HELLO_BASE64,
    });
  });

  it("generates, imports, and returns a composer attachment for inline display", async () => {
    const file = importedFile();
    const deps: GenerateChatImageDeps = {
      generate: vi.fn().mockResolvedValue(pngImage()),
      importImageBytes: vi.fn().mockResolvedValue(file),
    };

    const result = await generateChatImage("a red bicycle", deps, "fal-ai/flux/dev", "image-req-1");

    expect(deps.generate).toHaveBeenCalledWith(
      "a red bicycle",
      "fal-ai/flux/dev",
      "image-req-1",
      undefined,
    );
    // The decoded bytes (not the base64) are imported into the workspace.
    const [name, bytes] = (deps.importImageBytes as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(name).toMatch(/^generated-image-\d+\.png$/);
    expect(Array.from(bytes as Uint8Array)).toEqual(HELLO_BYTES);

    expect(result.status).toBe("ok");
    if (result.status !== "ok") return;
    // The returned attachment is the exact shape a pasted image produces, so it
    // renders inline through the existing display path.
    expect(result.attachment.kind).toBe("image");
    expect(result.attachment.status).toBe("imported");
    expect(result.attachment.workspacePath).toBe(file.path);
    expect(result.file).toBe(file);
    expect(parseImageDataUrl(result.dataUrl)).not.toBeNull();
  });

  it("falls back to the default model when none is passed", async () => {
    const deps: GenerateChatImageDeps = {
      generate: vi.fn().mockResolvedValue(pngImage()),
      importImageBytes: vi.fn().mockResolvedValue(importedFile()),
      defaultModel: () => "fal-ai/flux/dev",
    };

    await generateChatImage("anything", deps);

    expect(deps.generate).toHaveBeenCalledWith(
      "anything",
      "fal-ai/flux/dev",
      expect.any(String),
      undefined,
    );
  });

  it("forwards a pinned safe-mode value to the generate call", async () => {
    const deps: GenerateChatImageDeps = {
      generate: vi.fn().mockResolvedValue(pngImage()),
      importImageBytes: vi.fn().mockResolvedValue(importedFile()),
    };

    await generateChatImage("anything", deps, "fal-ai/flux/dev", "image-req-1", false);

    expect(deps.generate).toHaveBeenCalledWith("anything", "fal-ai/flux/dev", "image-req-1", false);
  });

  it("rejects a blank prompt without calling the backend", async () => {
    const deps: GenerateChatImageDeps = {
      generate: vi.fn(),
      importImageBytes: vi.fn(),
    };

    const result = await generateChatImage("   ", deps);

    expect(result).toEqual({
      status: "error",
      message: "Enter a prompt to generate an image.",
    });
    expect(deps.generate).not.toHaveBeenCalled();
  });

  it("surfaces a generation failure as an error without importing", async () => {
    const deps: GenerateChatImageDeps = {
      generate: vi.fn().mockRejectedValue(new Error("upstream_provider_failed")),
      importImageBytes: vi.fn(),
    };

    const result = await generateChatImage("anything", deps);

    expect(result).toEqual({
      status: "error",
      message: "upstream_provider_failed",
    });
    expect(deps.importImageBytes).not.toHaveBeenCalled();
  });

  it("rejects a non-image result before it reaches the display path", async () => {
    const deps: GenerateChatImageDeps = {
      generate: vi.fn().mockResolvedValue({
        imageBase64: HELLO_BASE64,
        mimeType: "text/plain",
        model: "fal-ai/flux/dev",
        provider: "fal",
      }),
      importImageBytes: vi.fn(),
    };

    const result = await generateChatImage("anything", deps);

    expect(result.status).toBe("error");
    expect(deps.importImageBytes).not.toHaveBeenCalled();
  });

  it("edits, imports, and returns a composer attachment for inline display", async () => {
    const file = importedFile();
    const deps: EditChatImageDeps = {
      readImageData: vi.fn().mockResolvedValue(`data:image/png;base64,${HELLO_BASE64}`),
      edit: vi.fn().mockResolvedValue(pngImage()),
      importImageBytes: vi.fn().mockResolvedValue(file),
    };

    const result = await editChatImage(file, "add a forest", deps);

    expect(deps.readImageData).toHaveBeenCalledWith(file.path);
    expect(deps.edit).toHaveBeenCalledWith(HELLO_BASE64, "add a forest", "image/png", undefined);
    const [name, bytes] = (deps.importImageBytes as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(name).toMatch(/^generated-image-\d+\.png$/);
    expect(Array.from(bytes as Uint8Array)).toEqual(HELLO_BYTES);

    expect(result.status).toBe("ok");
    if (result.status !== "ok") return;
    expect(result.attachment.kind).toBe("image");
    expect(result.file).toBe(file);
    expect(parseImageDataUrl(result.dataUrl)).not.toBeNull();
  });

  it("rejects an unreadable source image without calling the edit endpoint", async () => {
    const deps: EditChatImageDeps = {
      readImageData: vi.fn().mockResolvedValue(null),
      edit: vi.fn(),
      importImageBytes: vi.fn(),
    };

    const result = await editChatImage(importedFile(), "add a forest", deps);

    expect(result).toEqual({
      status: "error",
      message: "Clovy couldn't read the source image.",
    });
    expect(deps.edit).not.toHaveBeenCalled();
    expect(deps.importImageBytes).not.toHaveBeenCalled();
  });
});
