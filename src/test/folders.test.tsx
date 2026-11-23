import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { createRef } from "react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MoveNoteToFolderDialog } from "../components/folders/MoveNoteToFolderDialog";
import { NotesList, type NotesListHandle } from "../components/notes-list/NotesList";
import { Sidebar } from "../components/sidebar/Sidebar";
import type { FolderDto, NoteListItemDto } from "../lib/tauri";

const now = "2026-05-19T10:00:00Z";

const notes: NoteListItemDto[] = [
  {
    id: "note-2",
    title: "Second",
    preview: "Second preview",
    processingStatus: "ready",
    folderIds: ["folder-1"],
    createdAt: "2026-05-19T11:00:00Z",
    updatedAt: "2026-05-19T11:00:00Z",
  },
  {
    id: "note-1",
    title: "",
    preview: "",
    processingStatus: "draft",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
  },
];

describe("folders UI", () => {
  it("renders the primary entries and opens Today from the sidebar", async () => {
    const user = userEvent.setup();
    const onChangeView = vi.fn();
    const { container } = render(
      <Sidebar
        notes={notes}
        activeView="notes"
        onChangeView={onChangeView}
        onSelectNote={vi.fn()}
      />,
    );

    expect(screen.getByRole("img", { name: "Clovy" })).toHaveClass("sidebar-brand-mark");
    expect(container.querySelector(".sidebar-brand .clovy-app-tile")).toBeNull();
    expect(screen.getByRole("button", { name: "Meeting notes" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Folders" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Sessions" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Meeting notes" }));
    await user.click(screen.getByRole("button", { name: "Today" }));

    expect(onChangeView).toHaveBeenCalledWith("notes");
    expect(onChangeView).toHaveBeenCalledWith("today");
  });

  it("shows notes with placeholders and selects notes", async () => {
    const user = userEvent.setup();
    const onSelectNote = vi.fn();
    const { container } = render(
      <NotesList
        notes={notes}
        onSelectNote={onSelectNote}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );
    const list = within(container.querySelector(".all-notes-list") as HTMLElement);

    expect(list.getByRole("button", { name: /^Second/ })).toBeInTheDocument();
    expect(screen.queryByText("Ideas")).not.toBeInTheDocument();

    await user.click(list.getByRole("button", { name: /^Second/ }));
    expect(onSelectNote).toHaveBeenCalledWith("note-2");
  });

  it("shows the recording row state only for the active recording note", () => {
    render(
      <NotesList
        notes={[
          {
            ...notes[0],
            id: "stale-note",
            title: "Stale take",
            preview: "",
            processingStatus: "recording",
          },
          {
            ...notes[1],
            id: "active-note",
            title: "Active take",
            preview: "",
            processingStatus: "recording",
          },
        ]}
        activeRecordingNoteId="active-note"
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: /Active take Recording/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Stale take Draft/ })).toBeInTheDocument();
    expect(screen.getAllByText("Recording")).toHaveLength(1);
  });

  it("keeps only one meeting actions menu open", async () => {
    const user = userEvent.setup();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    const secondActions = screen.getByRole("button", {
      name: "Actions for Second",
    });
    const newMeetingActions = screen.getByRole("button", {
      name: "Actions for New note",
    });

    await user.click(secondActions);

    expect(screen.getAllByRole("menu")).toHaveLength(1);
    expect(secondActions).toHaveAttribute("aria-expanded", "true");

    await user.click(newMeetingActions);

    expect(screen.getAllByRole("menu")).toHaveLength(1);
    expect(secondActions).toHaveAttribute("aria-expanded", "false");
    expect(newMeetingActions).toHaveAttribute("aria-expanded", "true");
    expect(screen.getAllByRole("menuitem", { name: "Move to project" })).toHaveLength(1);
  });

  it("positions the meeting actions menu inside the viewport", async () => {
    const user = userEvent.setup();
    const originalInnerWidth = window.innerWidth;
    const originalInnerHeight = window.innerHeight;

    try {
      Object.defineProperty(window, "innerWidth", {
        configurable: true,
        value: 1000,
      });
      Object.defineProperty(window, "innerHeight", {
        configurable: true,
        value: 760,
      });
      render(
        <NotesList
          notes={notes}
          onSelectNote={vi.fn()}
          onCreateNote={vi.fn()}
          onOpenMoveDialog={vi.fn()}
          onOpenMoveNotes={vi.fn()}
          onDeleteNote={vi.fn()}
          onDeleteNotes={vi.fn()}
        />,
      );

      const actions = screen.getByRole("button", {
        name: "Actions for Second",
      });
      actions.getBoundingClientRect = vi.fn(
        () =>
          ({
            x: 900,
            y: 700,
            left: 900,
            right: 924,
            top: 700,
            bottom: 724,
            width: 24,
            height: 24,
            toJSON: () => ({}),
          }) as DOMRect,
      );

      await user.click(actions);

      const menu = screen.getByRole("menu");
      expect(menu).toHaveStyle({ right: "104px", top: "622px" });
    } finally {
      Object.defineProperty(window, "innerWidth", {
        configurable: true,
        value: originalInnerWidth,
      });
      Object.defineProperty(window, "innerHeight", {
        configurable: true,
        value: originalInnerHeight,
      });
    }
  });

  it("labels future-dated notes explicitly", () => {
    render(
      <NotesList
        notes={[
          {
            ...notes[0],
            id: "future-note",
            updatedAt: new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString(),
          },
        ]}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    expect(screen.getByText("Future")).toBeInTheDocument();
  });

  it("shows empty state with create action", () => {
    render(
      <NotesList
        notes={[]}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    expect(screen.getByText("Capture your first meeting")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create your first note" })).toBeInTheDocument();
  });

  it("bulk deletes selected meetings from the main list", async () => {
    const user = userEvent.setup();
    const onDeleteNotes = vi.fn();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={onDeleteNotes}
      />,
    );

    expect(screen.queryByRole("button", { name: "Select" })).toBeNull();
    expect(screen.getByRole("checkbox", { name: "Select Second" })).toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("checkbox", { name: "Select New note" }));
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete notes" }));

    expect(onDeleteNotes).toHaveBeenCalledWith(["note-2", "note-1"]);

    // A confirmed delete clears the selection and slides the bar out.
    const bar = screen.getByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveAttribute("data-exit", "slide");
    fireEvent.animationEnd(bar, {
      animationName: "meetings-bulk-bar-out-slide",
    });
    expect(screen.queryByRole("toolbar", { name: "Selection" })).toBeNull();
  });

  it("opens the move dialog with every selected meeting", async () => {
    const user = userEvent.setup();
    const onOpenMoveNotes = vi.fn();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={onOpenMoveNotes}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("checkbox", { name: "Select New note" }));
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(onOpenMoveNotes).toHaveBeenCalledWith(["note-2", "note-1"]);
  });

  it("can clear selected meetings after a bulk move succeeds", async () => {
    const user = userEvent.setup();
    const notesListRef = createRef<NotesListHandle>();
    render(
      <NotesList
        ref={notesListRef}
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("checkbox", { name: "Select New note" }));

    expect(screen.getByRole("toolbar", { name: "Selection" })).toHaveTextContent("2 selected");

    act(() => notesListRef.current?.resetSelection());

    const bar = screen.getByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveAttribute("data-exit", "slide");
    fireEvent.animationEnd(bar, {
      animationName: "meetings-bulk-bar-out-slide",
    });
    expect(screen.queryByRole("toolbar", { name: "Selection" })).toBeNull();
  });

  it("selects all visible meetings after one meeting is selected", async () => {
    const user = userEvent.setup();
    const onDeleteNotes = vi.fn();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={onDeleteNotes}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("button", { name: "Select all" }));
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Delete notes" }));

    expect(onDeleteNotes).toHaveBeenCalledWith(["note-2", "note-1"]);
  });

  it("deselects all visible meetings after all visible meetings are selected", async () => {
    const user = userEvent.setup();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("button", { name: "Select all" }));
    await user.click(screen.getByRole("button", { name: "Deselect all" }));

    // Deselect all empties the selection, so the bar fades out and unmounts
    // once its exit animation ends. jsdom never fires animation events, so
    // drive it manually.
    const bar = screen.getByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveAttribute("data-exit", "fade");
    fireEvent.animationEnd(bar, {
      animationName: "meetings-bulk-bar-out-fade",
    });

    expect(screen.queryByRole("toolbar", { name: "Selection" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.getByRole("checkbox", { name: "Select Second" })).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Select New note" })).not.toBeChecked();
  });

  it("slides the bar out and unmounts it after clearing the selection", async () => {
    const user = userEvent.setup();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("button", { name: "Clear selection" }));

    // The × is a deliberate dismiss, so the bar slides out before unmounting.
    const bar = screen.getByRole("toolbar", { name: "Selection" });
    expect(bar).toHaveAttribute("data-exit", "slide");
    fireEvent.animationEnd(bar, {
      animationName: "meetings-bulk-bar-out-slide",
    });

    expect(screen.queryByRole("toolbar", { name: "Selection" })).toBeNull();
  });

  it("cancels the exit when a note is reselected mid-animation", async () => {
    const user = userEvent.setup();
    render(
      <NotesList
        notes={notes}
        onSelectNote={vi.fn()}
        onCreateNote={vi.fn()}
        onOpenMoveDialog={vi.fn()}
        onOpenMoveNotes={vi.fn()}
        onDeleteNote={vi.fn()}
        onDeleteNotes={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("checkbox", { name: "Select Second" }));
    await user.click(screen.getByRole("button", { name: "Clear selection" }));
    expect(screen.getByRole("toolbar", { name: "Selection" })).toHaveAttribute(
      "data-exit",
      "slide",
    );

    // Re-selecting mid-exit shows the live bar again.
    await user.click(screen.getByRole("checkbox", { name: "Select New note" }));
    const bar = screen.getByRole("toolbar", { name: "Selection" });
    expect(bar).not.toHaveAttribute("data-exit");
    expect(within(bar).getByText("1 selected")).toBeInTheDocument();
  });
});

describe("MoveNoteToFolderDialog", () => {
  const moveFolders: FolderDto[] = [
    {
      id: "folder-1",
      name: "Alpha",
      memoryDisabled: false,
      createdAt: now,
      updatedAt: now,
    },
    {
      id: "folder-2",
      name: "Beta",
      memoryDisabled: false,
      createdAt: now,
      updatedAt: now,
    },
  ];

  it("moves every selected note to the picked project", async () => {
    const user = userEvent.setup();
    const onSetFolder = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    const onMoved = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={notes}
        folders={moveFolders}
        onSetFolder={onSetFolder}
        onMoved={onMoved}
      />,
    );

    expect(screen.getByRole("heading", { name: "Move 2 meeting notes" })).toBeInTheDocument();

    await user.click(screen.getByRole("option", { name: /Beta/ }));
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(onSetFolder).toHaveBeenCalledTimes(2);
    expect(onSetFolder).toHaveBeenNthCalledWith(1, "note-2", "folder-2");
    expect(onSetFolder).toHaveBeenNthCalledWith(2, "note-1", "folder-2");
    expect(onMoved).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps single-note copy and excludes its current project", async () => {
    render(
      <MoveNoteToFolderDialog
        open
        onClose={vi.fn()}
        notes={[notes[0]]}
        folders={moveFolders}
        onSetFolder={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Move meeting note" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /Alpha/ })).toBeNull();
    expect(screen.getByRole("option", { name: /Beta/ })).toBeInTheDocument();
  });

  it("lists the current project checked and unfiles the note when clicked", async () => {
    const user = userEvent.setup();
    const onRemoveFolder = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    const onMoved = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={[notes[0]]}
        folders={moveFolders}
        onSetFolder={vi.fn()}
        onRemoveFolder={onRemoveFolder}
        onMoved={onMoved}
      />,
    );

    const current = screen.getByRole("option", { name: "Remove from Alpha" });
    expect(current).toHaveAttribute("aria-selected", "true");

    await user.click(current);

    expect(onRemoveFolder).toHaveBeenCalledWith("note-2", "folder-1");
    expect(onMoved).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps the current project out of the list without an onRemoveFolder handler", () => {
    render(
      <MoveNoteToFolderDialog
        open
        onClose={vi.fn()}
        notes={[notes[0]]}
        folders={moveFolders}
        onSetFolder={vi.fn()}
      />,
    );

    expect(screen.queryByRole("option", { name: /Remove from/ })).toBeNull();
    expect(screen.queryByRole("option", { name: /Alpha/ })).toBeNull();
  });

  it("creates a project from the search query and files the note in it", async () => {
    const user = userEvent.setup();
    const created: FolderDto = {
      id: "folder-new",
      name: "Roadmap",
      memoryDisabled: false,
      createdAt: now,
      updatedAt: now,
    };
    const onCreateFolder = vi.fn().mockResolvedValue(created);
    const onSetFolder = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    const onMoved = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={[notes[1]]}
        folders={moveFolders}
        onSetFolder={onSetFolder}
        onCreateFolder={onCreateFolder}
        onMoved={onMoved}
      />,
    );

    await user.type(screen.getByPlaceholderText("Search or create project"), "Roadmap");
    await user.click(screen.getByRole("button", { name: "Create “Roadmap”" }));

    expect(onCreateFolder).toHaveBeenCalledWith("Roadmap");
    expect(onSetFolder).toHaveBeenCalledWith("note-1", "folder-new");
    expect(onMoved).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("hides the create row when the query matches an existing project name", async () => {
    const user = userEvent.setup();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={vi.fn()}
        notes={[notes[1]]}
        folders={moveFolders}
        onSetFolder={vi.fn()}
        onCreateFolder={vi.fn()}
      />,
    );

    await user.type(screen.getByPlaceholderText("Search or create project"), "alpha");

    expect(screen.queryByRole("button", { name: /^Create/ })).toBeNull();
    expect(screen.getByRole("option", { name: /Alpha/ })).toBeInTheDocument();
  });

  it("lets the user create their first project inline via Enter", async () => {
    const user = userEvent.setup();
    const created: FolderDto = {
      id: "folder-new",
      name: "Client work",
      memoryDisabled: false,
      createdAt: now,
      updatedAt: now,
    };
    const onCreateFolder = vi.fn().mockResolvedValue(created);
    const onSetFolder = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={[notes[1]]}
        folders={[]}
        onSetFolder={onSetFolder}
        onCreateFolder={onCreateFolder}
      />,
    );

    expect(screen.getByText("No projects yet. Type a name to create one.")).toBeInTheDocument();

    await user.type(screen.getByPlaceholderText("Search or create project"), "Client work{Enter}");

    expect(onCreateFolder).toHaveBeenCalledWith("Client work");
    expect(onSetFolder).toHaveBeenCalledWith("note-1", "folder-new");
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps the dialog open when assignment fails after creating the project", async () => {
    const user = userEvent.setup();
    const created: FolderDto = {
      id: "folder-new",
      name: "Roadmap",
      memoryDisabled: false,
      createdAt: now,
      updatedAt: now,
    };
    const onCreateFolder = vi.fn().mockResolvedValue(created);
    const onSetFolder = vi.fn().mockRejectedValue(new Error("db locked"));
    const onClose = vi.fn();
    const onMoved = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={[notes[1]]}
        folders={[]}
        onSetFolder={onSetFolder}
        onCreateFolder={onCreateFolder}
        onMoved={onMoved}
      />,
    );

    await user.type(screen.getByPlaceholderText("Search or create project"), "Roadmap");
    await user.click(screen.getByRole("button", { name: "Create “Roadmap”" }));

    expect(onSetFolder).toHaveBeenCalledWith("note-1", "folder-new");
    expect(onMoved).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("keeps the dialog open when project creation fails", async () => {
    const user = userEvent.setup();
    const onCreateFolder = vi.fn().mockResolvedValue(undefined);
    const onSetFolder = vi.fn();
    const onClose = vi.fn();
    render(
      <MoveNoteToFolderDialog
        open
        onClose={onClose}
        notes={[notes[1]]}
        folders={[]}
        onSetFolder={onSetFolder}
        onCreateFolder={onCreateFolder}
      />,
    );

    await user.type(screen.getByPlaceholderText("Search or create project"), "Roadmap");
    await user.click(screen.getByRole("button", { name: "Create “Roadmap”" }));

    expect(onCreateFolder).toHaveBeenCalledWith("Roadmap");
    expect(onSetFolder).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
