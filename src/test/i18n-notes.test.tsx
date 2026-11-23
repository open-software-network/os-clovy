import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { userFacingFailureMessage } from "../components/note-editor/NoteFailureBanner";
import { NotesList } from "../components/notes-list/NotesList";
import { ProjectSettingsDialog } from "../components/folders/ProjectSettingsDialog";
import { applyInterfaceLocale } from "../i18n/locale";
import type { FolderDto, NoteListItemDto } from "../lib/tauri";

vi.mock("../lib/tauri", () => ({
  listMemories: vi.fn().mockResolvedValue([{ id: "m1" }, { id: "m2" }]),
  memorySettings: vi.fn().mockResolvedValue({ enabled: true }),
  setFolderInstructions: vi.fn(),
  setFolderMemoryDisabled: vi.fn(),
}));

function note(id: string, title: string): NoteListItemDto {
  return {
    id,
    title,
    preview: "",
    processingStatus: "ready",
    folderIds: [],
    updatedAt: new Date().toISOString(),
  } as unknown as NoteListItemDto;
}

function renderNotesList(notes: NoteListItemDto[]) {
  return render(
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
}

const folder = {
  id: "f1",
  name: "Roadmap",
  description: "",
  createdAt: new Date().toISOString(),
  updatedAt: new Date().toISOString(),
} as unknown as FolderDto;

describe("notes i18n (English default)", () => {
  it("renders the meeting notes list in English", () => {
    renderNotesList([]);
    expect(screen.getByRole("heading", { name: "Meeting notes" })).toBeInTheDocument();
    expect(screen.getByText("Capture your first meeting")).toBeInTheDocument();
  });
});

describe("notes i18n (pt-BR)", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));
  afterEach(() => applyInterfaceLocale("en"));

  it("translates the notes list, row labels and the plural selection count", () => {
    renderNotesList([note("n1", "Sync semanal"), note("n2", "")]);
    expect(screen.getByRole("heading", { name: /Notas de reunião/ })).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Buscar")).toBeInTheDocument();
    // An untitled note falls back to the translated placeholder; the user's
    // own title is never translated.
    expect(screen.getByText("Nova nota", { selector: ".folder-note-title" })).toBeInTheDocument();
    expect(screen.getByText("Sync semanal")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox", { name: "Selecionar Sync semanal" }));
    expect(screen.getByText("1 selecionada")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: "Selecionar Nova nota" }));
    expect(screen.getByText("2 selecionadas")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Desmarcar tudo" })).toBeInTheDocument();
    expect(screen.getByRole("toolbar", { name: "Seleção" })).toBeInTheDocument();
  });

  it("switches language live after render", () => {
    applyInterfaceLocale("en");
    renderNotesList([]);
    expect(screen.getByText("Capture your first meeting")).toBeInTheDocument();
    act(() => applyInterfaceLocale("pt-BR"));
    expect(screen.getByText("Capture sua primeira reunião")).toBeInTheDocument();
  });

  it("translates failure guidance and keeps the source prefix localized", () => {
    expect(userFacingFailureMessage("Microphone: authorization_denied")).toBe(
      "Microfone: O serviço está ocupado agora. Aguarde um minuto e tente de novo.",
    );
  });

  it("translates project settings with an interpolated memory count", async () => {
    render(
      <ProjectSettingsDialog
        open
        folder={folder}
        onClose={vi.fn()}
        onSaveDetails={vi.fn()}
        onFolderUpdated={vi.fn()}
        onManageMemory={vi.fn()}
        onRequestDelete={vi.fn()}
      />,
    );
    expect(screen.getByText("Configurações do projeto")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Excluir projeto" })).toBeInTheDocument();
    expect(screen.getByLabelText("Instruções do projeto")).toBeInTheDocument();
    expect(await screen.findByText("2 memórias salvas")).toBeInTheDocument();
  });
});
