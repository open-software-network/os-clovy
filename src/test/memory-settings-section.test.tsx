import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MemorySettingsSection } from "../components/settings/MemorySettingsSection";
import type { FolderDto, MemoryDto } from "../lib/tauri";
import { setCurrentDataPartitionName } from "../lib/data-partition";

const mocks = vi.hoisted(() => ({
  listMemories: vi.fn(),
  memorySettings: vi.fn(),
  setMemoryEnabled: vi.fn(),
  createMemory: vi.fn(),
  updateMemory: vi.fn(),
  deleteMemory: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

vi.mock("../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/tauri")>();
  return { ...actual, ...mocks };
});

const stamp = "2026-07-01T00:00:00Z";

const folders: FolderDto[] = [
  { id: "project-a", name: "Alpha", memoryDisabled: false, createdAt: stamp, updatedAt: stamp },
  { id: "project-b", name: "Beta", memoryDisabled: false, createdAt: stamp, updatedAt: stamp },
];

// The manager shows every memory — global and per-project.
const memories: MemoryDto[] = [
  {
    id: "global-user",
    content: "Use concise answers",
    source: "user",
    createdAt: "2026-07-03T00:00:00Z",
    updatedAt: "2026-07-03T00:00:00Z",
  },
  {
    id: "global-agent",
    content: "Prefers metric units",
    source: "agent",
    createdAt: "2026-07-04T00:00:00Z",
    updatedAt: "2026-07-04T00:00:00Z",
  },
  {
    id: "alpha",
    folderId: "project-a",
    content: "Launch day is Friday",
    source: "agent",
    createdAt: "2026-07-05T00:00:00Z",
    updatedAt: "2026-07-05T00:00:00Z",
  },
];

let persistedMemories: MemoryDto[] = [];
let persistedEnabled = true;

beforeEach(() => {
  vi.clearAllMocks();
  setCurrentDataPartitionName("default");
  mocks.listen.mockResolvedValue(() => {});
  persistedMemories = memories.map((memory) => ({ ...memory }));
  persistedEnabled = true;
  mocks.listMemories.mockImplementation(async () => persistedMemories);
  mocks.memorySettings.mockImplementation(async () => ({ enabled: persistedEnabled }));
  mocks.setMemoryEnabled.mockImplementation(async (enabled: boolean) => {
    persistedEnabled = enabled;
    return { enabled };
  });
  mocks.createMemory.mockImplementation(
    async ({ folderId, content }: { folderId?: string; content: string }) => {
      const created: MemoryDto = {
        id: "created",
        folderId,
        content,
        source: "user",
        createdAt: "2026-07-06T00:00:00Z",
        updatedAt: "2026-07-06T00:00:00Z",
      };
      persistedMemories = [created, ...persistedMemories];
      return created;
    },
  );
  mocks.updateMemory.mockImplementation(async (id: string, content: string) => {
    const memory = persistedMemories.find((candidate) => candidate.id === id);
    if (!memory) throw new Error("Missing test memory");
    const updated = { ...memory, content };
    persistedMemories = persistedMemories.map((candidate) =>
      candidate.id === id ? updated : candidate,
    );
    return updated;
  });
  mocks.deleteMemory.mockImplementation(async (id: string) => {
    persistedMemories = persistedMemories.filter((memory) => memory.id !== id);
  });
});

describe("MemorySettingsSection", () => {
  it("refreshes the visible memory after a native mutation and ignores other partitions", async () => {
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    render(<MemorySettingsSection folders={folders} />);
    expect(await screen.findByText("Use concise answers")).toBeInTheDocument();
    mocks.listMemories.mockResolvedValue([
      { ...memories[0], content: "Updated by the bound conversation" },
    ]);
    act(() => changed?.({ payload: { profile: "other", kind: "memories" } }));
    expect(screen.queryByText("Updated by the bound conversation")).not.toBeInTheDocument();
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    expect(await screen.findByText("Updated by the bound conversation")).toBeInTheDocument();
    expect(screen.queryByText("Use concise answers")).not.toBeInTheDocument();
  });

  it("does not hydrate a different partition after an invalidation read waits", async () => {
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");
    let finish: ((items: MemoryDto[]) => void) | undefined;
    mocks.listMemories.mockImplementationOnce(
      () =>
        new Promise<MemoryDto[]>((resolve) => {
          finish = resolve;
        }),
    );
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    await act(async () => {
      setCurrentDataPartitionName("other");
      finish?.([{ ...memories[0], content: "Must not cross the partition switch" }]);
    });
    expect(screen.queryByText("Must not cross the partition switch")).not.toBeInTheDocument();
  });

  it("keeps a newer event snapshot when the initial memory read resolves late", async () => {
    const initial = deferred<MemoryDto[]>();
    mocks.listMemories
      .mockReturnValueOnce(initial.promise)
      .mockResolvedValue([{ ...memories[0], content: "Current persisted fact" }]);
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    render(<MemorySettingsSection folders={folders} />);
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    await screen.findByText("Current persisted fact");
    await act(async () => initial.resolve(memories));
    expect(screen.getByText("Current persisted fact")).toBeInTheDocument();
    expect(screen.queryByText("Use concise answers")).not.toBeInTheDocument();
  });

  it("does not restore on from an older event after the local setting is saved off", async () => {
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");
    const oldSettings = deferred<{ enabled: boolean }>();
    mocks.memorySettings.mockReturnValueOnce(oldSettings.promise);
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    await user.click(screen.getByRole("switch", { name: "Let Clovy remember things" }));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Let Clovy remember things" })).not.toBeChecked(),
    );
    await act(async () => oldSettings.resolve({ enabled: true }));
    expect(screen.getByRole("switch", { name: "Let Clovy remember things" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
    expect(persistedEnabled).toBe(false);
  });

  it.each([
    "delete",
    "edit",
    "create",
  ] as const)("keeps a successful local %s after an older native refresh completes", async (operation) => {
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");
    const oldSettings = deferred<{ enabled: boolean }>();
    mocks.memorySettings.mockReturnValueOnce(oldSettings.promise);
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    if (operation === "create") {
      await user.click(screen.getByRole("button", { name: "Add memory" }));
      const dialog = screen.getByRole("dialog", { name: "Add memory" });
      await user.type(within(dialog).getByRole("textbox", { name: "Memory" }), "New local fact");
      await user.click(within(dialog).getByRole("button", { name: "Add memory" }));
      await screen.findByText("New local fact");
    } else {
      const row = screen.getByText("Use concise answers").closest(".memory-row") as HTMLElement;
      await user.click(
        within(row).getByRole("button", {
          name: operation === "edit" ? "Edit memory" : "Delete memory",
        }),
      );
      const dialog = screen.getByRole("dialog", {
        name: operation === "edit" ? "Edit memory" : "Delete memory?",
      });
      if (operation === "edit") {
        const input = within(dialog).getByRole("textbox", { name: "Memory" });
        await user.clear(input);
        await user.type(input, "New local wording");
      }
      await user.click(
        within(dialog).getByRole("button", {
          name: operation === "edit" ? "Save changes" : "Delete",
        }),
      );
      await waitFor(() =>
        expect(screen.queryByText("Use concise answers")).not.toBeInTheDocument(),
      );
    }
    await act(async () => oldSettings.resolve({ enabled: true }));
    if (operation === "create") expect(screen.getByText("New local fact")).toBeInTheDocument();
    else {
      expect(screen.queryByText("Use concise answers")).not.toBeInTheDocument();
      if (operation === "edit") expect(screen.getByText("New local wording")).toBeInTheDocument();
    }
    if (operation === "create")
      expect(persistedMemories.find((memory) => memory.id === "created")?.content).toBe(
        "New local fact",
      );
    else
      expect(persistedMemories.find((memory) => memory.id === "global-user")?.content).toBe(
        operation === "edit" ? "New local wording" : undefined,
      );
  });

  it.each([
    "edit",
    "create",
    "delete",
    "toggle",
  ] as const)("reconciles a persisted MCP forget after local %s fails without losing its error or draft", async (operation) => {
    persistedMemories = [{ ...memories[0] }];
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (_name, callback) => {
      changed = callback;
      return () => {};
    });
    mocks.updateMemory.mockImplementation(async (id: string) => {
      if (!persistedMemories.some((memory) => memory.id === id))
        throw { code: "memory_not_found", message: "Memory was not found." };
      throw new Error("Expected the persisted MCP deletion");
    });
    mocks.deleteMemory.mockImplementation(async (id: string) => {
      if (!persistedMemories.some((memory) => memory.id === id))
        throw { code: "memory_not_found", message: "Memory was not found." };
      throw new Error("Expected the persisted MCP deletion");
    });
    mocks.createMemory.mockImplementation(async () => {
      if (!persistedEnabled)
        throw { code: "memory_disabled", message: "Memory is disabled for this scope." };
      throw new Error("Expected the persisted MCP setting");
    });
    mocks.setMemoryEnabled.mockImplementation(async (enabled: boolean) => {
      persistedEnabled = enabled;
      throw new Error("Native runtime enforcement failed after persistence");
    });
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");
    if (operation === "create") {
      await user.click(screen.getByRole("button", { name: "Add memory" }));
      await user.type(screen.getByRole("textbox", { name: "Memory" }), "Keep this unsaved draft");
    } else if (operation !== "toggle") {
      await user.click(
        screen.getByRole("button", {
          name: operation === "edit" ? "Edit memory" : "Delete memory",
        }),
      );
      if (operation === "edit") {
        await user.clear(screen.getByRole("textbox", { name: "Memory" }));
        await user.type(screen.getByRole("textbox", { name: "Memory" }), "Keep this unsaved draft");
      }
    }
    // Native mutation commits first; only its visible invalidation response waits.
    persistedMemories = [];
    if (operation === "create") persistedEnabled = false;
    const nativeResponse = deferred<MemoryDto[]>();
    mocks.listMemories.mockReturnValueOnce(nativeResponse.promise);
    act(() => changed?.({ payload: { profile: "default", kind: "memories" } }));
    if (operation === "toggle")
      await user.click(screen.getByRole("switch", { name: "Let Clovy remember things" }));
    else
      await user.click(
        within(
          screen.getByRole("dialog", {
            name:
              operation === "edit"
                ? "Edit memory"
                : operation === "create"
                  ? "Add memory"
                  : "Delete memory?",
          }),
        ).getByRole("button", {
          name:
            operation === "edit"
              ? "Save changes"
              : operation === "create"
                ? "Add memory"
                : "Delete",
        }),
      );
    const expectedError =
      operation === "toggle"
        ? "Your memory setting was saved, but Clovy could not finish applying it. Quit and reopen Clovy before starting another agent run."
        : operation === "create"
          ? "Memory is disabled for this scope."
          : "Memory was not found.";
    await screen.findAllByText(expectedError);
    await act(async () => nativeResponse.resolve([]));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Use concise answers" })).not.toBeInTheDocument(),
    );
    expect(screen.getAllByText(expectedError).length).toBeGreaterThan(0);
    expect(persistedMemories).toEqual([]);
    if (operation === "edit" || operation === "create")
      expect(screen.getByRole("textbox", { name: "Memory" })).toHaveValue(
        "Keep this unsaved draft",
      );
    if (operation === "toggle" || operation === "create") {
      expect(screen.getByRole("switch", { name: "Let Clovy remember things" })).not.toBeChecked();
      expect(persistedEnabled).toBe(false);
    }
  });

  it("keeps a persisted toggle and its error while the failed-mutation list reconciliation waits", async () => {
    mocks.setMemoryEnabled.mockImplementationOnce(async (enabled: boolean) => {
      persistedEnabled = enabled;
      throw new Error("Runtime enforcement failed after persistence");
    });
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");
    const listing = deferred<MemoryDto[]>();
    mocks.listMemories.mockImplementation(() => listing.promise);
    await user.click(screen.getByRole("switch", { name: "Let Clovy remember things" }));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Let Clovy remember things" })).not.toBeChecked(),
    );
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Your memory setting was saved");
    expect(persistedEnabled).toBe(false);
    await act(async () => listing.resolve(persistedMemories));
    expect(screen.getByRole("switch", { name: "Let Clovy remember things" })).not.toBeChecked();
    expect(screen.getByRole("alert")).toHaveTextContent("Your memory setting was saved");
  });

  it("allows the user to add the first memory from the empty state", async () => {
    mocks.listMemories.mockResolvedValueOnce([]);
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);

    expect(await screen.findByText("Nothing remembered yet")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add memory" }));

    const addDialog = screen.getByRole("dialog", { name: "Add memory" });
    await user.type(within(addDialog).getByRole("textbox", { name: "Memory" }), "Call Sam");
    await user.click(within(addDialog).getByRole("button", { name: "Add memory" }));

    await waitFor(() =>
      expect(mocks.createMemory).toHaveBeenCalledWith({
        content: "Call Sam",
        source: "user",
      }),
    );
    expect(await screen.findByText("Call Sam")).toBeInTheDocument();
  });

  it("lists every memory with project tags, sources, and a count", async () => {
    render(<MemorySettingsSection folders={folders} />);

    expect(await screen.findByText("Launch day is Friday")).toBeInTheDocument();
    // Fetches all memories, global and per-project.
    expect(mocks.listMemories).toHaveBeenCalledWith(undefined, true);
    // Total count sits in the section heading pill, matching Agent / Skills.
    const heading = screen.getByRole("heading", { level: 2, name: /Saved memories/ });
    expect(within(heading).getByText("3")).toBeInTheDocument();
    // Project tag only on the scoped memory; un-scoped rows show no chip.
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.queryByText("General")).toBeNull();
    expect(screen.getByText("Added by you")).toBeInTheDocument();
    expect(screen.getAllByText("Added by Clovy")).toHaveLength(2);
  });

  it("filters the list by search query", async () => {
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Launch day is Friday");

    await user.type(screen.getByRole("searchbox", { name: "Search memories" }), "metric");
    expect(screen.getByText("Prefers metric units")).toBeInTheDocument();
    expect(screen.queryByText("Launch day is Friday")).toBeNull();
  });

  it("filters the list to a single project", async () => {
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Launch day is Friday");

    await user.click(screen.getByRole("button", { name: "Filter memories by project" }));
    await user.click(screen.getByRole("option", { name: /Alpha/ }));

    expect(screen.getByText("Launch day is Friday")).toBeInTheDocument();
    expect(screen.queryByText("Use concise answers")).toBeNull();
  });

  it("pre-filters to the project passed as initialFolderFilter", async () => {
    render(<MemorySettingsSection folders={folders} initialFolderFilter="project-a" />);

    expect(await screen.findByText("Launch day is Friday")).toBeInTheDocument();
    expect(screen.queryByText("Use concise answers")).toBeNull();
  });

  it("can change a project-scoped new memory back to General", async () => {
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} initialFolderFilter="project-a" />);
    await screen.findByText("Launch day is Friday");

    await user.click(screen.getByRole("button", { name: "Add memory" }));
    const addDialog = screen.getByRole("dialog", { name: "Add memory" });
    expect(within(addDialog).getByRole("button", { name: "Memory project" })).toHaveTextContent(
      "Alpha",
    );

    await user.click(within(addDialog).getByRole("button", { name: "Memory project" }));
    await user.click(screen.getByRole("option", { name: "General" }));
    await user.type(within(addDialog).getByRole("textbox", { name: "Memory" }), "Call Sam");
    await user.click(within(addDialog).getByRole("button", { name: "Add memory" }));

    await waitFor(() =>
      expect(mocks.createMemory).toHaveBeenCalledWith({
        content: "Call Sam",
        source: "user",
      }),
    );
  });

  it("adds a memory, then edits and deletes through the bindings", async () => {
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);
    await screen.findByText("Use concise answers");

    await user.click(screen.getByRole("button", { name: "Add memory" }));
    const addDialog = screen.getByRole("dialog", { name: "Add memory" });
    await user.type(within(addDialog).getByRole("textbox", { name: "Memory" }), "Call Sam");
    await user.click(within(addDialog).getByRole("button", { name: "Add memory" }));
    await waitFor(() =>
      expect(mocks.createMemory).toHaveBeenCalledWith({
        content: "Call Sam",
        source: "user",
      }),
    );

    const conciseRow = screen.getByText("Use concise answers").closest(".memory-row");
    await user.click(
      within(conciseRow as HTMLElement).getByRole("button", { name: "Edit memory" }),
    );
    const editDialog = screen.getByRole("dialog", { name: "Edit memory" });
    const editField = within(editDialog).getByRole("textbox", { name: "Memory" });
    await user.clear(editField);
    await user.type(editField, "Use very concise answers");
    await user.click(within(editDialog).getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(mocks.updateMemory).toHaveBeenCalledWith("global-user", "Use very concise answers"),
    );

    const updatedRow = screen.getByText("Use very concise answers").closest(".memory-row");
    await user.click(
      within(updatedRow as HTMLElement).getByRole("button", { name: "Delete memory" }),
    );
    const deleteDialog = screen.getByRole("dialog", { name: "Delete memory?" });
    await user.click(within(deleteDialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(mocks.deleteMemory).toHaveBeenCalledWith("global-user"));
  });

  it("wires the global toggle and keeps saved memories inspectable while off", async () => {
    mocks.memorySettings.mockResolvedValueOnce({ enabled: false });
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);

    expect(await screen.findByText("Use concise answers")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
    expect(screen.getAllByRole("button", { name: "Edit memory" })[0]).toBeDisabled();
    expect(
      screen.getByText(
        "Memory is off. Saved memories remain visible, but Clovy cannot add or update them.",
      ),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Let Clovy remember things" }));
    expect(mocks.setMemoryEnabled).toHaveBeenCalledWith(true);
    await waitFor(() => expect(screen.getByRole("button", { name: "Add memory" })).toBeEnabled());
  });

  it("shows persisted off after native enforcement fails without rolling the setting back", async () => {
    mocks.memorySettings
      .mockResolvedValueOnce({ enabled: true })
      .mockResolvedValueOnce({ enabled: false });
    mocks.setMemoryEnabled.mockRejectedValueOnce(new Error("Runtime enforcement failed"));
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);

    const toggle = await screen.findByRole("switch", { name: "Let Clovy remember things" });
    expect(toggle).toBeChecked();
    await user.click(toggle);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Your memory setting was saved, but Clovy could not finish applying it. Quit and reopen Clovy before starting another agent run.",
    );
    expect(screen.queryByText("Runtime enforcement failed")).toBeNull();
    await waitFor(() => expect(toggle).not.toBeChecked());
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
    expect(mocks.memorySettings).toHaveBeenCalledTimes(2);
  });

  it("shows persisted on after native restart fails without rolling the setting back", async () => {
    mocks.memorySettings
      .mockResolvedValueOnce({ enabled: false })
      .mockResolvedValueOnce({ enabled: true });
    mocks.setMemoryEnabled.mockRejectedValueOnce(new Error("Runtime restart failed"));
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);

    const toggle = await screen.findByRole("switch", { name: "Let Clovy remember things" });
    expect(toggle).not.toBeChecked();
    await user.click(toggle);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Your memory setting was saved, but Clovy could not finish applying it. Quit and reopen Clovy before starting another agent run.",
    );
    expect(screen.queryByText("Runtime restart failed")).toBeNull();
    await waitFor(() => expect(toggle).toBeChecked());
    expect(screen.getByRole("button", { name: "Add memory" })).toBeEnabled();
    expect(mocks.memorySettings).toHaveBeenCalledTimes(2);
  });

  it("keeps the switch disabled until the authoritative toggle response returns", async () => {
    let resolveToggle: ((settings: { enabled: boolean }) => void) | undefined;
    mocks.setMemoryEnabled.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveToggle = (settings) => {
            persistedEnabled = settings.enabled;
            resolve(settings);
          };
        }),
    );
    const user = userEvent.setup();
    render(<MemorySettingsSection folders={folders} />);

    const toggle = await screen.findByRole("switch", { name: "Let Clovy remember things" });
    await user.click(toggle);
    expect(toggle).toBeDisabled();

    await user.click(toggle);
    expect(mocks.setMemoryEnabled).toHaveBeenCalledTimes(1);

    resolveToggle?.({ enabled: false });
    await waitFor(() => expect(toggle).not.toBeDisabled());
    expect(toggle).not.toBeChecked();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}
