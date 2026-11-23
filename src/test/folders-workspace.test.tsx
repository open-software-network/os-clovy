import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FoldersWorkspace } from "../components/folders/FoldersWorkspace";
import { Sidebar } from "../components/sidebar/Sidebar";
import { LEGACY_NOTE_DND_MIME, NOTE_DND_MIME } from "../lib/dnd";
import type { FolderDto, NoteListItemDto } from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  listMemories: vi.fn(),
  memorySettings: vi.fn(),
  setFolderInstructions: vi.fn(),
  setFolderMemoryDisabled: vi.fn(),
  updateMemory: vi.fn(),
  deleteMemory: vi.fn(),
}));

vi.mock("../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/tauri")>();

  return {
    ...actual,
    listMemories: mocks.listMemories,
    memorySettings: mocks.memorySettings,
    setFolderInstructions: mocks.setFolderInstructions,
    setFolderMemoryDisabled: mocks.setFolderMemoryDisabled,
    updateMemory: mocks.updateMemory,
    deleteMemory: mocks.deleteMemory,
  };
});

const now = "2026-05-19T10:00:00Z";

const folders: FolderDto[] = [
  { id: "folder-1", name: "Ideas", memoryDisabled: false, createdAt: now, updatedAt: now },
  {
    id: "folder-2",
    name: "Work",
    description: "Client projects in flight",
    memoryDisabled: false,
    createdAt: now,
    updatedAt: now,
  },
];

const notes: NoteListItemDto[] = [
  {
    id: "note-1",
    title: "Roadmap",
    preview: "Q3 priorities",
    processingStatus: "ready",
    folderIds: ["folder-2"],
    createdAt: now,
    updatedAt: now,
  },
  {
    id: "note-2",
    title: "Loose thought",
    preview: "",
    processingStatus: "draft",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
  },
];

function stubNavigatorPlatform(platform: string, userAgent: string) {
  const ownPlatform = Object.getOwnPropertyDescriptor(navigator, "platform");
  const ownUserAgent = Object.getOwnPropertyDescriptor(navigator, "userAgent");
  Object.defineProperty(navigator, "platform", {
    configurable: true,
    get: () => platform,
  });
  Object.defineProperty(navigator, "userAgent", {
    configurable: true,
    get: () => userAgent,
  });
  return () => {
    if (ownPlatform) {
      Object.defineProperty(navigator, "platform", ownPlatform);
    } else {
      Reflect.deleteProperty(navigator, "platform");
    }
    if (ownUserAgent) {
      Object.defineProperty(navigator, "userAgent", ownUserAgent);
    } else {
      Reflect.deleteProperty(navigator, "userAgent");
    }
  };
}

function baseProps() {
  return {
    folders,
    notes,
    sessions: [],
    sessionFolderIds: {},
    selectedFolderId: undefined as string | undefined,
    onSelectFolder: vi.fn(),
    onCreateFolder: vi.fn(),
    onRenameFolder: vi.fn(),
    onFolderUpdated: vi.fn(),
    onDeleteFolder: vi.fn(),
    onCreateNote: vi.fn(),
    onCreateSession: vi.fn(),
    onSelectNote: vi.fn(),
    onAssignNoteToFolder: vi.fn(async () => undefined),
    onRemoveNoteFromFolder: vi.fn(),
    onOpenMoveDialog: vi.fn(),
    onDeleteNote: vi.fn(),
    onSelectSession: vi.fn(),
    onAssignSessionToFolder: vi.fn(async () => undefined),
    onRemoveSessionFromFolder: vi.fn(),
    onOpenSessionMoveDialog: vi.fn(),
    onManageProjectMemory: vi.fn(),
  };
}

beforeEach(() => {
  vi.clearAllMocks();

  mocks.listMemories.mockResolvedValue([]);
  mocks.memorySettings.mockResolvedValue({ enabled: true });
  mocks.setFolderInstructions.mockImplementation(
    async (folderId: string, instructions?: string) => {
      const folder = folders.find((candidate) => candidate.id === folderId);
      if (!folder) throw new Error("Missing test folder");
      return { ...folder, instructions };
    },
  );
  mocks.setFolderMemoryDisabled.mockImplementation(async (folderId: string, disabled: boolean) => {
    const folder = folders.find((candidate) => candidate.id === folderId);
    if (!folder) throw new Error("Missing test folder");
    return { ...folder, memoryDisabled: disabled };
  });
  mocks.updateMemory.mockImplementation(async (id: string, content: string) => ({
    id,
    content,
    folderId: "folder-2",
    source: "user",
    createdAt: now,
    updatedAt: now,
  }));
  mocks.deleteMemory.mockResolvedValue(undefined);
});

describe("Sidebar primary navigation", () => {
  it("shows Notes and Projects in primary navigation", async () => {
    const user = userEvent.setup();
    const onChangeView = vi.fn();
    render(
      <Sidebar
        notes={notes}
        activeView="notes"
        onChangeView={onChangeView}
        onSelectNote={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: /Folders/ })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Meeting notes" }));
    expect(onChangeView).toHaveBeenCalledWith("notes");
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(onChangeView).toHaveBeenCalledWith("folders");
  });

  it("opens the command prompt with Command-K", async () => {
    render(
      <Sidebar notes={notes} activeView="notes" onChangeView={vi.fn()} onSelectNote={vi.fn()} />,
    );

    expect(screen.getByText("⌘K")).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "k", metaKey: true });

    const prompt = screen.getByRole("dialog", { name: "Search" });
    const search = within(prompt).getByRole("textbox", { name: "Search" });
    await waitFor(() => expect(search).toHaveFocus());
    expect(
      within(prompt).getByPlaceholderText("Search meeting notes or jump to..."),
    ).toBeInTheDocument();
    expect(within(prompt).getByText("Recents")).toBeInTheDocument();
    expect(within(prompt).getByText("Roadmap")).toBeInTheDocument();
    expect(within(prompt).getByText("Quick actions")).toBeInTheDocument();
    expect(within(prompt).queryByText("New session")).not.toBeInTheDocument();

    fireEvent.keyDown(search, { key: "Escape" });

    expect(screen.queryByRole("dialog", { name: "Search" })).toBeNull();
  });

  it("opens the command prompt with Command-K while the sidebar is collapsed", async () => {
    const { container } = render(
      <Sidebar
        notes={notes}
        activeView="notes"
        collapsed
        onChangeView={vi.fn()}
        onSelectNote={vi.fn()}
      />,
    );

    fireEvent.keyDown(window, { key: "k", metaKey: true });

    const prompt = screen.getByRole("dialog", { name: "Search" });
    const search = within(prompt).getByRole("textbox", { name: "Search" });
    await waitFor(() => expect(search).toHaveFocus());

    // The collapsed sidebar is `display: none`, so the prompt must live outside
    // the sidebar subtree (portaled to the body) to stay visible.
    const sidebar = container.querySelector(".sidebar");
    expect(sidebar).not.toBeNull();
    expect(sidebar?.contains(prompt)).toBe(false);
    expect(document.body.contains(prompt)).toBe(true);
  });

  it("closes the command prompt on Escape even when a result row is focused", async () => {
    render(
      <Sidebar notes={notes} activeView="notes" onChangeView={vi.fn()} onSelectNote={vi.fn()} />,
    );

    fireEvent.keyDown(window, { key: "k", metaKey: true });
    const prompt = screen.getByRole("dialog", { name: "Search" });

    // Move focus off the input onto a result row (the prompt has no focus trap,
    // so Esc must still close from here — the "can't get stuck" guarantee).
    const row = within(prompt).getByRole("button", { name: /Roadmap/ });
    row.focus();
    expect(row).toHaveFocus();

    fireEvent.keyDown(row, { key: "Escape" });

    expect(screen.queryByRole("dialog", { name: "Search" })).toBeNull();
  });

  it("shows and accepts the Windows command prompt shortcut", async () => {
    const restoreNavigator = stubNavigatorPlatform(
      "Win32",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    );
    try {
      render(
        <Sidebar notes={notes} activeView="notes" onChangeView={vi.fn()} onSelectNote={vi.fn()} />,
      );

      expect(screen.getByText("Ctrl K")).toBeInTheDocument();

      fireEvent.keyDown(window, { key: "k", ctrlKey: true });

      const prompt = screen.getByRole("dialog", { name: "Search" });
      const search = within(prompt).getByRole("textbox", { name: "Search" });
      await waitFor(() => expect(search).toHaveFocus());
    } finally {
      restoreNavigator();
    }
  });

  it("opens the command prompt when clicking the sidebar search", async () => {
    const user = userEvent.setup();
    render(
      <Sidebar notes={notes} activeView="notes" onChangeView={vi.fn()} onSelectNote={vi.fn()} />,
    );

    await user.click(screen.getByRole("searchbox", { name: "Search" }));

    const prompt = screen.getByRole("dialog", { name: "Search" });
    await waitFor(() =>
      expect(within(prompt).getByRole("textbox", { name: "Search" })).toHaveFocus(),
    );
  });

  it("renders settings as a sidebar footer action", async () => {
    const user = userEvent.setup();
    const onChangeView = vi.fn();
    render(
      <Sidebar
        notes={notes}
        activeView="notes"
        onChangeView={onChangeView}
        onSelectNote={vi.fn()}
      />,
    );

    // The settings entry point is the footer row.
    const settingsButton = screen.getByRole("button", { name: "Settings" });
    expect(settingsButton.closest(".sidebar-footer")).not.toBeNull();

    await user.click(settingsButton);
    expect(onChangeView).toHaveBeenCalledWith("settings");
  });

  it("opens dictation history from the primary nav", async () => {
    const user = userEvent.setup();
    const onChangeView = vi.fn();
    render(
      <Sidebar
        notes={notes}
        activeView="notes"
        onChangeView={onChangeView}
        onSelectNote={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Dictation" }));
    expect(onChangeView).toHaveBeenCalledWith("dictation");
  });

  it("renders grouped icon settings navigation without focusing the identity row", async () => {
    const user = userEvent.setup();
    const onSettingsTabChange = vi.fn();
    const onExitSettings = vi.fn();
    render(
      <Sidebar
        notes={notes}
        activeView="settings"
        settingsTab="appearance"
        onSettingsTabChange={onSettingsTabChange}
        onExitSettings={onExitSettings}
        onChangeView={vi.fn()}
        onSelectNote={vi.fn()}
      />,
    );

    expect(screen.queryByLabelText("Clovy")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back to app" }));
    expect(onExitSettings).toHaveBeenCalledTimes(1);

    expect(screen.getByRole("navigation", { name: "Personal settings" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Audio settings" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "AI settings" })).toBeInTheDocument();

    const appearanceButton = screen.getByRole("button", { name: "Appearance" });
    expect(appearanceButton).toHaveAttribute("data-active", "true");
    expect(screen.queryByRole("button", { name: "Privacy" })).toBeNull();
    expect(screen.getByRole("button", { name: "Shortcuts" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Permissions" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Shortcuts" }));
    expect(onSettingsTabChange).toHaveBeenCalledWith("shortcuts");

    expect(screen.getByRole("button", { name: "Settings" })).not.toHaveAttribute("data-active");
  });
});

describe("FoldersWorkspace — list view", () => {
  it("renders folder cards without a virtual all-notes folder", () => {
    render(<FoldersWorkspace {...baseProps()} />);

    expect(screen.getByRole("heading", { name: "Projects" })).toBeInTheDocument();
    expect(screen.queryByText("All notes")).toBeNull();
    // No virtual "Notes" card — the side nav already lists all notes.
    expect(screen.queryByRole("button", { name: /^Notes/ })).toBeNull();
    expect(screen.queryByText("Roadmap")).toBeNull();
    expect(screen.getByText("Ideas")).toBeInTheDocument();
    expect(screen.getByText("Work")).toBeInTheDocument();
    const workCard = screen.getByText("Work").closest("article");
    expect(workCard).not.toBeNull();
    // Description preferred over note count when present.
    expect(
      within(workCard as HTMLElement).getByText("Client projects in flight"),
    ).toBeInTheDocument();
    const ideasCard = screen.getByText("Ideas").closest("article");
    expect(within(ideasCard as HTMLElement).getByText(/0 meeting notes/)).toBeInTheDocument();
  });

  it("opens the create dialog and submits name + description", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} />);

    await user.click(screen.getByRole("button", { name: /New project/ }));
    expect(screen.getByRole("dialog", { name: /Create project/ })).toBeInTheDocument();

    await user.type(screen.getByLabelText("Name"), "Personal");
    await user.type(screen.getByLabelText("Description"), "Side projects");
    await user.click(screen.getByRole("button", { name: /Create project/ }));

    expect(props.onCreateFolder).toHaveBeenCalledWith("Personal", "Side projects");
  });

  it("filters folders by search query", async () => {
    const user = userEvent.setup();
    render(<FoldersWorkspace {...baseProps()} />);

    await user.type(screen.getByPlaceholderText("Search"), "work");
    expect(screen.queryByText("Ideas")).toBeNull();
    expect(screen.getByText("Work")).toBeInTheDocument();
  });

  it("opens a folder when its card is clicked", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} />);

    const ideasCard = screen.getByText("Ideas").closest("article");
    await user.click(within(ideasCard as HTMLElement).getByText("Ideas"));
    expect(props.onSelectFolder).toHaveBeenCalledWith("folder-1");
  });

  it("normalizes legacy multi-folder notes when dropped on an assigned folder", () => {
    const props = baseProps();
    render(
      <FoldersWorkspace
        {...props}
        notes={[{ ...notes[0], folderIds: ["folder-1", "folder-2"] }]}
      />,
    );

    fireEvent.drop(screen.getByRole("button", { name: "Open Ideas" }), {
      dataTransfer: {
        types: [NOTE_DND_MIME],
        getData: () => "note-1",
      },
    });

    expect(props.onAssignNoteToFolder).toHaveBeenCalledWith("note-1", "folder-1");
  });

  it("accepts the June-era note drag type as a compatibility alias", () => {
    const props = baseProps();
    render(<FoldersWorkspace {...props} />);

    fireEvent.drop(screen.getByRole("button", { name: "Open Ideas" }), {
      dataTransfer: {
        types: [LEGACY_NOTE_DND_MIME],
        getData: (type: string) => (type === LEGACY_NOTE_DND_MIME ? "note-1" : ""),
      },
    });

    expect(props.onAssignNoteToFolder).toHaveBeenCalledWith("note-1", "folder-1");
  });

  it("clears drop highlight when a drag is cancelled", async () => {
    render(<FoldersWorkspace {...baseProps()} />);
    const card = screen.getByRole("button", { name: "Open Ideas" });

    fireEvent.dragEnter(card, {
      dataTransfer: {
        types: [NOTE_DND_MIME],
      },
    });
    expect(card).toHaveAttribute("data-drop-active", "true");

    fireEvent.dragEnd(document);

    await waitFor(() => expect(card).not.toHaveAttribute("data-drop-active"));
  });

  it("keeps delete confirmation open until async delete resolves", async () => {
    const user = userEvent.setup();
    let resolveDelete: (() => void) | undefined;
    const props = {
      ...baseProps(),
      onDeleteFolder: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            resolveDelete = resolve;
          }),
      ),
    };
    render(<FoldersWorkspace {...props} />);

    await user.click(screen.getByRole("button", { name: /Actions for Ideas/ }));
    await user.click(screen.getByRole("menuitem", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete project" }));

    expect(screen.getByRole("dialog", { name: /Delete "Ideas"/ })).toBeInTheDocument();

    resolveDelete?.();

    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: /Delete "Ideas"/ })).not.toBeInTheDocument(),
    );
  });

  it("keeps delete confirmation open when async delete fails", async () => {
    const user = userEvent.setup();
    const props = {
      ...baseProps(),
      onDeleteFolder: vi.fn(() => Promise.reject(new Error("Nope"))),
    };
    render(<FoldersWorkspace {...props} />);

    await user.click(screen.getByRole("button", { name: /Actions for Ideas/ }));
    await user.click(screen.getByRole("menuitem", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete project" }));

    expect(screen.getByRole("dialog", { name: /Delete "Ideas"/ })).toBeInTheDocument();
  });
});

describe("FoldersWorkspace — detail view", () => {
  it("renders the folder via sticky header and surfaces description + meta", () => {
    render(<FoldersWorkspace {...baseProps()} selectedFolderId="folder-2" />);

    // Folder name shows as the editable title.
    expect(screen.getByRole("button", { name: /Rename project/ })).toHaveTextContent("Work");
    expect(screen.getByText("Client projects in flight")).toBeInTheDocument();
    expect(screen.getByText("Roadmap")).toBeInTheDocument();
  });

  it("enters edit mode on a single click of the title", async () => {
    const user = userEvent.setup();
    render(<FoldersWorkspace {...baseProps()} selectedFolderId="folder-1" />);

    await user.click(screen.getByRole("button", { name: /Rename project/ }));
    // The serif title is replaced by an input that auto-selects its value.
    expect(document.activeElement).toBeInstanceOf(HTMLInputElement);
    expect((document.activeElement as HTMLInputElement).value).toBe("Ideas");
  });

  it("returns to the list via the back button in the sticky bar", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-1" />);

    await user.click(screen.getByRole("button", { name: /back to projects/i }));
    expect(props.onSelectFolder).toHaveBeenCalledWith(undefined);
  });

  it("returns to the provided source when opened from a note", async () => {
    const user = userEvent.setup();
    const props = {
      ...baseProps(),
      folderBackTarget: {
        label: "Back to Test",
        onBack: vi.fn(),
      },
    };
    render(<FoldersWorkspace {...props} selectedFolderId="folder-1" />);

    await user.click(screen.getByRole("button", { name: /back to test/i }));
    expect(props.folderBackTarget.onBack).toHaveBeenCalled();
    expect(props.onSelectFolder).not.toHaveBeenCalled();
  });

  it("renders empty-state actions and triggers create-note", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-1" />);

    // The empty surface is visual-only — no helper text — just the
    // primary action and "Add existing note" when other notes exist.
    await user.click(screen.getByRole("button", { name: /^New meeting note$/ }));
    expect(props.onCreateNote).toHaveBeenCalledWith("folder-1");
  });

  it("creates a project note from the header add menu", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-2" />);

    await user.click(screen.getByRole("button", { name: "Add to project" }));
    await user.click(screen.getByRole("menuitem", { name: "New meeting note" }));
    expect(props.onCreateNote).toHaveBeenCalledWith("folder-2");
  });

  it("removes a note from the folder via its row overflow menu", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-2" />);

    await user.click(screen.getByRole("button", { name: /Actions for Roadmap/ }));
    await user.click(screen.getByRole("menuitem", { name: /Remove from project/ }));
    expect(props.onRemoveNoteFromFolder).toHaveBeenCalledWith("note-1", "folder-2");
  });

  it("prefills the instructions field in Project settings from the folder", async () => {
    const user = userEvent.setup();
    render(
      <FoldersWorkspace
        {...baseProps()}
        folders={[{ ...folders[1], instructions: "Answer in French" }]}
        selectedFolderId="folder-2"
      />,
    );

    // Instructions live in Project settings, not on the surface.
    expect(screen.queryByRole("button", { name: /^Instructions/ })).toBeNull();

    await openProjectSettings(user, "Work");
    const dialog = screen.getByRole("dialog", { name: "Project settings" });
    expect(within(dialog).getByRole("textbox", { name: "Project instructions" })).toHaveValue(
      "Answer in French",
    );
  });

  it("saves project instructions from Project settings", async () => {
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-2" />);

    await openProjectSettings(user, "Work");
    const dialog = screen.getByRole("dialog", { name: "Project settings" });
    await user.type(
      within(dialog).getByRole("textbox", { name: "Project instructions" }),
      "Keep answers concise",
    );
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.setFolderInstructions).toHaveBeenCalledWith("folder-2", "Keep answers concise"),
    );
    expect(props.onFolderUpdated).toHaveBeenCalledWith(
      expect.objectContaining({ instructions: "Keep answers concise" }),
    );
  });

  it("disables Save when instructions exceed the 4000 character limit", async () => {
    const user = userEvent.setup();
    render(<FoldersWorkspace {...baseProps()} selectedFolderId="folder-2" />);

    await openProjectSettings(user, "Work");
    const dialog = screen.getByRole("dialog", { name: "Project settings" });
    const textarea = within(dialog).getByRole("textbox", { name: "Project instructions" });
    fireEvent.change(textarea, { target: { value: "x".repeat(4_001) } });

    expect(within(dialog).getByText("4001 / 4000 characters")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("shows a memory count + toggle and deep-links to the manager from Project settings", async () => {
    mocks.listMemories.mockResolvedValueOnce([
      {
        id: "memory-1",
        folderId: "folder-2",
        content: "The launch is Friday",
        source: "agent",
        createdAt: now,
        updatedAt: now,
      },
    ]);
    const user = userEvent.setup();
    const props = baseProps();
    render(<FoldersWorkspace {...props} selectedFolderId="folder-2" />);

    await openProjectSettings(user, "Work");
    const dialog = screen.getByRole("dialog", { name: "Project settings" });

    // Memory is a count + link here, not an inline list.
    expect(await within(dialog).findByText("1 memory saved")).toBeInTheDocument();
    expect(within(dialog).queryByText("The launch is Friday")).toBeNull();

    await user.click(
      within(dialog).getByRole("switch", { name: "Remember things in this project" }),
    );
    expect(mocks.setFolderMemoryDisabled).toHaveBeenCalledWith("folder-2", true);
    expect(props.onFolderUpdated).toHaveBeenCalledWith(
      expect.objectContaining({ memoryDisabled: true }),
    );

    // "Manage memories" hands off to the full manager, scoped to this project.
    await user.click(within(dialog).getByRole("button", { name: "Manage memories" }));
    expect(props.onManageProjectMemory).toHaveBeenCalledWith("folder-2");
  });

  it("shows global memory off and disables the project toggle in Project settings", async () => {
    mocks.memorySettings.mockResolvedValueOnce({ enabled: false });
    const user = userEvent.setup();
    render(<FoldersWorkspace {...baseProps()} selectedFolderId="folder-2" />);

    await openProjectSettings(user, "Work");
    const dialog = screen.getByRole("dialog", { name: "Project settings" });
    expect(
      await within(dialog).findByText("Memory is turned off in Settings > Memory."),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("switch", { name: "Remember things in this project" }),
    ).toBeDisabled();
  });
});

async function openProjectSettings(user: ReturnType<typeof userEvent.setup>, folderName: string) {
  await user.click(screen.getByRole("button", { name: `Actions for ${folderName}` }));
  await user.click(screen.getByRole("menuitem", { name: "Project settings" }));
}
