import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Suspense, startTransition, useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { ProjectSettingsDialog } from "../components/folders/ProjectSettingsDialog";
import type { FolderDto } from "../lib/tauri";

const fixture = vi.hoisted(() => ({
  listener: undefined as
    | undefined
    | ((event: { payload: { profile: string; kind: string } }) => void),
  renderedB: false,
  blockB: true,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (
      _name: string,
      listener: (event: { payload: { profile: string; kind: string } }) => void,
    ) => {
      fixture.listener = listener;
      return () => {};
    },
  ),
}));
vi.mock("../lib/tauri", () => ({
  listMemories: vi.fn(async (id: string) =>
    Array.from({ length: id === "A" ? 1 : 3 }, (_, index) => ({ id: String(index) })),
  ),
  memorySettings: vi.fn(async () => ({ enabled: true })),
  setFolderInstructions: vi.fn(),
  setFolderMemoryDisabled: vi.fn(),
}));

const folder = (id: string): FolderDto => ({
  id,
  name: `Project ${id}`,
  memoryDisabled: false,
  createdAt: "2026-01-01",
  updatedAt: "2026-01-01",
});
const suspended = new Promise<never>(() => {});
function PauseB({ id }: { id: string }) {
  if (id === "B" && fixture.blockB) {
    fixture.renderedB = true;
    throw suspended;
  }
  return null;
}
function Scenario() {
  const [id, setId] = useState("A");
  return (
    <>
      <button type="button" onClick={() => startTransition(() => setId("B"))}>
        Render B
      </button>
      <button type="button" onClick={() => setId("A")}>
        Keep A
      </button>
      <button
        type="button"
        onClick={() => {
          fixture.blockB = false;
          setId("B");
        }}
      >
        Commit B
      </button>
      <Suspense fallback={<div>Loading</div>}>
        <ProjectSettingsDialog
          open
          folder={folder(id)}
          onClose={() => {}}
          onSaveDetails={() => {}}
          onFolderUpdated={() => {}}
          onManageMemory={() => {}}
          onRequestDelete={() => {}}
        />
        <PauseB id={id} />
      </Suspense>
    </>
  );
}

describe("project refresh during interrupted render", () => {
  it("refreshes the committed project while the next project is suspended and abandoned", async () => {
    fixture.listener = undefined;
    fixture.renderedB = false;
    fixture.blockB = true;
    render(<Scenario />);
    await waitFor(() => expect(fixture.listener).toBeDefined());
    await waitFor(() =>
      expect(document.querySelector(".project-settings-memory-count")?.textContent).toMatch(/^1\b/),
    );
    fireEvent.click(screen.getByText("Render B"));
    await waitFor(() => expect(fixture.renderedB).toBe(true));
    await act(async () =>
      fixture.listener?.({ payload: { profile: "default", kind: "memories" } }),
    );
    await waitFor(() =>
      expect(document.querySelector(".project-settings-memory-count")?.textContent).toMatch(/^1\b/),
    );
    fireEvent.click(screen.getByText("Keep A"));
    expect(screen.getByDisplayValue("Project A")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Commit B"));
    await waitFor(() => expect(screen.getByDisplayValue("Project B")).toBeInTheDocument());
    await act(async () =>
      fixture.listener?.({ payload: { profile: "default", kind: "memories" } }),
    );
    await waitFor(() =>
      expect(document.querySelector(".project-settings-memory-count")?.textContent).toMatch(/^3\b/),
    );
  });
});
