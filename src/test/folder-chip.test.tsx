import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { NoteEditor } from "../components/note-editor/NoteEditor";
import type { FolderDto, NoteDto } from "../lib/tauri";

const now = "2026-05-19T10:00:00Z";

function note(): NoteDto {
  return {
    id: "note-1",
    title: "Untitled",
    preview: "",
    processingStatus: "ready",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
    activeTab: "notes",
  };
}

function baseProps(folders: FolderDto[]) {
  return {
    note: note(),
    folders,
    sourceMode: "microphonePlusSystem" as const,
    checkingSourceReadiness: false,
    onTitleChange: vi.fn(),
    onContentChange: vi.fn(),
    onFlushNote: vi.fn(),
    onSourceModeChange: vi.fn(),
    onEnableSystemAudio: vi.fn(),
    onEnableMicrophone: vi.fn(),
    microphoneBlocked: false,
    onStartRecording: vi.fn(),
    onPauseRecording: vi.fn(),
    onResumeRecording: vi.fn(),
    onFinishRecording: vi.fn(),
    onRetry: vi.fn(),
    onRecoverRecording: vi.fn(),
    onDiscardRecording: vi.fn(),
    onAssignFolder: vi.fn(),
    onRemoveFolder: vi.fn(),
    onCreateAndAssignFolder: vi.fn(),
    onNavigateToFolders: vi.fn(),
    onNavigateToFolder: vi.fn(),
    onTabChange: vi.fn(),
  };
}

describe("Folder chip — move-to-folder popover", () => {
  it("shows 'Project' label when nothing is assigned", () => {
    render(<NoteEditor {...baseProps([])} />);
    expect(screen.getByRole("button", { name: /^Project/ })).toBeInTheDocument();
  });

  it("filters folders by the search query", async () => {
    const user = userEvent.setup();
    render(
      <NoteEditor
        {...baseProps([
          { id: "f1", name: "Ideas", memoryDisabled: false, createdAt: now, updatedAt: now },
          { id: "f2", name: "Work", memoryDisabled: false, createdAt: now, updatedAt: now },
        ])}
      />,
    );

    await user.click(screen.getByRole("button", { name: /^Project/ }));
    const input = screen.getByPlaceholderText("Search or create project");
    await user.type(input, "wor");

    expect(screen.queryByText("Ideas")).toBeNull();
    expect(screen.getByText("Work")).toBeInTheDocument();
  });

  it("opens the assigned folder without opening the assignment popover", async () => {
    const user = userEvent.setup();
    const onNavigateToFolder = vi.fn();
    render(
      <NoteEditor
        {...baseProps([
          { id: "f1", name: "Ideas", memoryDisabled: false, createdAt: now, updatedAt: now },
        ])}
        note={{ ...note(), folderIds: ["f1"] }}
        onNavigateToFolder={onNavigateToFolder}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Open Ideas" }));

    expect(onNavigateToFolder).toHaveBeenCalledWith("f1");
    expect(screen.queryByPlaceholderText("Search or create project")).toBeNull();
  });

  it("offers 'Create' when no existing folder matches", async () => {
    const user = userEvent.setup();
    const props = baseProps([
      { id: "f1", name: "Ideas", memoryDisabled: false, createdAt: now, updatedAt: now },
    ]);
    render(<NoteEditor {...props} />);

    await user.click(screen.getByRole("button", { name: /^Project/ }));
    await user.type(screen.getByPlaceholderText("Search or create project"), "Personal");

    const create = screen.getByRole("button", { name: /Create.*Personal/ });
    await user.click(create);
    expect(props.onCreateAndAssignFolder).toHaveBeenCalledWith("Personal");
  });
});
