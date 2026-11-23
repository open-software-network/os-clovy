import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentSettingsSection } from "../components/settings/AgentSettingsSection";
import { ClovyPersonalitySettingsSection } from "../components/settings/ClovyPersonalitySettingsSection";
import type { AgentSkillDto } from "../lib/agent-runtime-contract";
import { onboardingArea, onboardingMood } from "../lib/onboarding";

const mocks = vi.hoisted(() => ({
  agentHudHide: vi.fn(),
  agentHudShow: vi.fn(),
  emit: vi.fn().mockResolvedValue(undefined),
  clovyPersona: vi.fn(),
  listAgentSkills: vi.fn(),
  setAgentSkillEnabled: vi.fn(),
  readAgentSkill: vi.fn(),
  updateAgentSkill: vi.fn(),
  setClovyPersona: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: mocks.emit,
  listen: mocks.listen,
}));

vi.mock("../lib/tauri", () => ({
  agentHudHide: mocks.agentHudHide,
  agentHudShow: mocks.agentHudShow,
  clovyPersona: mocks.clovyPersona,
  listAgentSkills: mocks.listAgentSkills,
  readAgentSkill: mocks.readAgentSkill,
  setAgentSkillEnabled: mocks.setAgentSkillEnabled,
  setClovyPersona: mocks.setClovyPersona,
  updateAgentSkill: mocks.updateAgentSkill,
}));

describe("AgentSettingsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listen.mockResolvedValue(() => {});
    localStorage.clear();
    mocks.listAgentSkills.mockResolvedValue([]);
    mocks.clovyPersona.mockResolvedValue({
      schemaVersion: 1,
      area: "personal",
      voice: 80,
      detail: 55,
      initiative: 70,
      humor: 45,
    });
    mocks.setClovyPersona.mockImplementation(async (request) => ({
      schemaVersion: 1,
      ...request,
    }));
  });

  it("keeps a native disable refresh when the initial skill load returns late", async () => {
    const oldLoad = deferred<(typeof skillFixture)[]>();
    mocks.listAgentSkills
      .mockReturnValueOnce(oldLoad.promise)
      .mockResolvedValue([{ ...skillFixture, enabled: false, configuredEnabled: false }]);
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (name, callback) => {
      if (name === "clovy-data-changed") changed = callback;
      return () => {};
    });
    render(<AgentSettingsSection />);
    act(() => changed?.({ payload: { profile: "default", kind: "skills" } }));
    expect(await screen.findByRole("switch", { name: "Enable Research" })).not.toBeChecked();
    await act(async () => oldLoad.resolve([skillFixture]));
    expect(screen.getByRole("switch", { name: "Enable Research" })).not.toBeChecked();
  });

  it("keeps a local disable when an older event skill catalog resolves late", async () => {
    let stored = skillFixture;
    mocks.listAgentSkills.mockImplementation(async () => [stored]);
    mocks.setAgentSkillEnabled.mockImplementation(async () => {
      stored = { ...stored, enabled: false, configuredEnabled: false };
      return stored;
    });
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (name, callback) => {
      if (name === "clovy-data-changed") changed = callback;
      return () => {};
    });
    const user = userEvent.setup();
    render(<AgentSettingsSection />);
    await screen.findByRole("switch", { name: "Disable Research" });
    const staleCatalog = deferred<(typeof skillFixture)[]>();
    mocks.listAgentSkills.mockReturnValueOnce(staleCatalog.promise);
    act(() => changed?.({ payload: { profile: "default", kind: "skills" } }));
    await user.click(screen.getByRole("switch", { name: "Disable Research" }));
    await screen.findByRole("switch", { name: "Enable Research" });
    await act(async () => staleCatalog.resolve([skillFixture]));
    expect(screen.getByRole("switch", { name: "Enable Research" })).not.toBeChecked();
  });

  it("persists off for a configured but unavailable skill and keeps a repaired skill inactive", async () => {
    const unavailable: AgentSkillDto = {
      ...skillFixture,
      enabled: false,
      configuredEnabled: true,
      readable: false,
      error: { code: "agent_skill_too_large", message: "Skill content exceeds 512 KiB." },
      description: "Unavailable skill: Skill content exceeds 512 KiB.",
    };
    let stored: AgentSkillDto = unavailable;
    mocks.listAgentSkills.mockImplementation(async () => [stored]);
    mocks.setAgentSkillEnabled.mockImplementation(async (_id, configuredEnabled) => {
      stored = { ...stored, configuredEnabled, enabled: configuredEnabled && stored.readable };
      return stored;
    });
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (name, callback) => {
      if (name === "clovy-data-changed") changed = callback;
      return () => {};
    });
    const user = userEvent.setup();
    render(<AgentSettingsSection />);
    expect(await screen.findByText(unavailable.description)).toBeInTheDocument();
    const toggle = screen.getByRole("switch", { name: "Disable Research" });
    expect(toggle).toBeChecked();
    await user.click(toggle);
    await waitFor(() => expect(mocks.setAgentSkillEnabled).toHaveBeenCalledWith("research", false));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Enable Research" })).not.toBeChecked(),
    );
    stored = {
      ...stored,
      readable: true,
      error: null,
      description: "Repaired instructions",
      enabled: stored.configuredEnabled,
    };
    act(() => changed?.({ payload: { profile: "default", kind: "skills" } }));
    await screen.findByText("Repaired instructions");
    expect(screen.getByRole("switch", { name: "Enable Research" })).not.toBeChecked();
    expect(stored.configuredEnabled).toBe(false);
    expect(stored.enabled).toBe(false);
  });

  it.each([
    "save",
    "toggle",
  ] as const)("reconciles a persisted MCP skill disable after local %s rejects and preserves the error and draft", async (operation) => {
    let stored: AgentSkillDto = { ...skillFixture };
    mocks.listAgentSkills.mockImplementation(async () => [stored]);
    mocks.readAgentSkill.mockImplementation(async () => ({ content: "Original instructions" }));
    mocks.updateAgentSkill.mockImplementation(async (_id: string, content: string) => {
      if (new TextEncoder().encode(content).byteLength > 512 * 1024)
        throw { code: "agent_skill_too_large", message: "Skill content exceeds 512 KiB." };
      throw new Error("Expected normal oversized-content rejection");
    });
    mocks.setAgentSkillEnabled.mockImplementation(async () => {
      if (!stored.editable)
        throw {
          code: "agent_skill_read_only",
          message: "User-global skills are read-only in Clovy.",
        };
      throw new Error("Expected a read-only skill after root discovery changed");
    });
    let changed: ((event: { payload: { profile: string; kind: string } }) => void) | undefined;
    mocks.listen.mockImplementation(async (name, callback) => {
      if (name === "clovy-data-changed") changed = callback;
      return () => {};
    });
    const user = userEvent.setup();
    render(<AgentSettingsSection />);
    await screen.findByRole("switch", { name: "Disable Research" });
    const draft = "x".repeat(512 * 1024 + 1);
    if (operation === "save") {
      await user.click(screen.getByRole("button", { name: "Edit" }));
      const editor = await screen.findByRole("textbox", { name: "Skill instructions" });
      fireEvent.change(editor, { target: { value: draft } });
    }
    stored = {
      ...stored,
      enabled: false,
      configuredEnabled: false,
      editable: operation !== "toggle",
      source: operation === "toggle" ? "user_global" : "managed",
    };
    const nativeResponse = deferred<AgentSkillDto[]>();
    mocks.listAgentSkills.mockReturnValueOnce(nativeResponse.promise);
    act(() => changed?.({ payload: { profile: "default", kind: "skills" } }));
    await user.click(
      screen.getByRole(operation === "save" ? "button" : "switch", {
        name: operation === "save" ? "Save skill" : "Disable Research",
      }),
    );
    await waitFor(() =>
      expect(
        operation === "save" ? mocks.updateAgentSkill : mocks.setAgentSkillEnabled,
      ).toHaveBeenCalled(),
    );
    await act(async () => nativeResponse.resolve([stored]));
    expect(await screen.findByRole("switch", { name: "Enable Research" })).not.toBeChecked();
    expect(screen.getByRole("alert")).toHaveTextContent(
      operation === "save"
        ? "Skill content exceeds 512 KiB."
        : "User-global skills are read-only in Clovy.",
    );
    expect(stored.configuredEnabled).toBe(false);
    expect(stored.enabled).toBe(false);
    if (operation === "save")
      expect(screen.getByRole("textbox", { name: "Skill instructions" })).toHaveValue(draft);
    else expect(screen.getByRole("switch", { name: "Enable Research" })).toBeDisabled();
  });

  it.each([
    "refused",
    "successful",
  ] as const)("scopes errors to a new %s Edit after cancelling a refused oversized draft", async (nextRead) => {
    const document = {
      name: "research",
      content: "Original instructions",
      path: "/tmp/clovy-t4-review4/research/SKILL.md",
      readOnly: false,
    };
    let documentExists = true;
    let delayRead = false;
    const readGate = deferred<void>();
    const catalogRepair = deferred<AgentSkillDto[]>();
    mocks.listAgentSkills.mockResolvedValue([skillFixture]);
    mocks.readAgentSkill.mockImplementation(async () => {
      if (delayRead) await readGate.promise;
      if (!documentExists) throw { code: "agent_skill_not_found", message: "Skill was not found." };
      return document;
    });
    mocks.updateAgentSkill.mockImplementation(async (_id: string, content: string) => {
      if (new TextEncoder().encode(content).byteLength > 512 * 1024)
        throw { code: "agent_skill_too_large", message: "Skill content exceeds 512 KiB." };
      throw new Error("Expected normal oversized-content refusal");
    });
    const user = userEvent.setup();
    render(<AgentSettingsSection />);
    await screen.findByRole("switch", { name: "Disable Research" });
    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = await screen.findByRole("textbox", { name: "Skill instructions" });
    const oversizedDraft = "x".repeat(512 * 1024 + 1);
    fireEvent.change(editor, { target: { value: oversizedDraft } });
    mocks.listAgentSkills.mockImplementation(() => catalogRepair.promise);
    await user.click(screen.getByRole("button", { name: "Save skill" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Skill content exceeds 512 KiB.");
    expect(editor).toHaveValue(oversizedDraft);
    await user.click(
      within(screen.getByRole("dialog", { name: "Edit Research" })).getByRole("button", {
        name: "Cancel",
      }),
    );
    expect(screen.queryByRole("dialog", { name: "Edit Research" })).not.toBeInTheDocument();

    documentExists = nextRead === "successful";
    delayRead = true;
    await user.click(screen.getByRole("button", { name: "Edit" }));
    // Starting another action is not evidence of success. Retain the actual
    // earlier failure until this read supplies its own result.
    expect(screen.getByRole("alert")).toHaveTextContent("Skill content exceeds 512 KiB.");
    expect(screen.queryByRole("dialog", { name: "Edit Research" })).not.toBeInTheDocument();
    await act(async () => readGate.resolve(undefined));
    const errorBeforeCatalogRepair = screen.queryByRole("alert")?.textContent;
    await act(async () => catalogRepair.resolve([skillFixture]));

    if (nextRead === "refused") {
      expect(errorBeforeCatalogRepair).toContain("Skill was not found.");
      expect(screen.getByRole("alert")).toHaveTextContent("Skill was not found.");
      expect(screen.getByRole("alert")).not.toHaveTextContent("Skill content exceeds 512 KiB.");
      expect(screen.queryByRole("dialog", { name: "Edit Research" })).not.toBeInTheDocument();
    } else {
      expect(errorBeforeCatalogRepair).toBeUndefined();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(screen.getByRole("textbox", { name: "Skill instructions" })).toHaveValue(
        document.content,
      );
    }
  });

  it.each([
    "success",
    "refusal",
  ] as const)("ignores an older Edit %s after the latest Edit has already settled", async (oldOutcome) => {
    type ReadDocument = { name: string; content: string; path: string; readOnly: boolean };
    const oldRead = deferred<ReadDocument>();
    const latestRead = deferred<ReadDocument>();
    const oldDocument = {
      name: "research",
      content: "Old document",
      path: "/tmp/clovy-t4-review4/research/SKILL.md",
      readOnly: false,
    };
    const latestDocument = {
      name: "research",
      content: "Latest persisted instructions",
      path: "/tmp/clovy-t4-review4/research/SKILL.md",
      readOnly: false,
    };
    mocks.listAgentSkills.mockResolvedValue([skillFixture]);
    let readResponse = oldRead.promise;
    mocks.readAgentSkill.mockImplementation(() => readResponse);
    const user = userEvent.setup();
    render(<AgentSettingsSection />);
    const edit = await screen.findByRole("button", { name: "Edit" });
    await user.click(edit);
    readResponse = latestRead.promise;
    await user.click(edit);

    await act(async () => {
      if (oldOutcome === "success")
        latestRead.reject({
          code: "agent_skill_not_found",
          message: "Skill was not found.",
        });
      else latestRead.resolve(latestDocument);
    });
    const latestError = screen.queryByRole("alert")?.textContent;
    const latestDraft = screen.queryByRole("textbox", { name: "Skill instructions" });
    await act(async () => {
      if (oldOutcome === "success") oldRead.resolve(oldDocument);
      else
        oldRead.reject({
          code: "agent_skill_read_failed",
          message: "Permission denied (os error 13)",
        });
    });

    if (oldOutcome === "success") {
      expect(latestError).toContain("Skill was not found.");
      expect(screen.getByRole("alert")).toHaveTextContent("Skill was not found.");
      expect(screen.queryByRole("dialog", { name: "Edit Research" })).not.toBeInTheDocument();
    } else {
      expect(latestError).toBeUndefined();
      expect(latestDraft).toHaveValue(latestDocument.content);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(screen.getByRole("textbox", { name: "Skill instructions" })).toHaveValue(
        latestDocument.content,
      );
    }
  });

  it("loads the current native personality into the Agent settings", async () => {
    render(<ClovyPersonalitySettingsSection />);

    expect(screen.getByRole("heading", { name: "Personality" })).toBeInTheDocument();
    // The heading renders before the native persona resolves; the radios only
    // exist once it has, so wait for them rather than racing the load.
    expect(await screen.findByRole("radio", { name: /Calm/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /Calm/ }).closest("label")).toHaveClass(
      "settings-card",
    );
    expect(screen.getByText("Take your time. What should we think through first?")).toHaveClass(
      "shimmer",
    );
    expect(document.querySelector(".settings-personality-option-check")).toBeNull();
    expect(screen.queryByText("Focus area")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
  });

  it("automatically saves a new personality while preserving the onboarding area", async () => {
    const user = userEvent.setup();
    render(<ClovyPersonalitySettingsSection />);
    await screen.findByRole("radio", { name: /Calm/ });

    await user.click(screen.getByRole("radio", { name: /Quick-witted/ }));

    expect(mocks.setClovyPersona).toHaveBeenCalledWith({
      area: "personal",
      voice: 85,
      detail: 70,
      initiative: 80,
      humor: 95,
    });
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(onboardingArea()).toBe("personal");
    expect(onboardingMood()).toBe("quick-witted");
  });

  it("restores the persisted personality when an automatic save fails", async () => {
    const user = userEvent.setup();
    mocks.setClovyPersona.mockRejectedValueOnce(new Error("Could not write persona settings"));
    render(<ClovyPersonalitySettingsSection />);
    await screen.findByRole("radio", { name: /Calm/ });

    await user.click(screen.getByRole("radio", { name: /Strategic/ }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not write persona settings");
    expect(screen.getByRole("radio", { name: /Calm/ })).toBeChecked();
    expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
    expect(onboardingArea()).toBeNull();
  });

  it("keeps a queued personality visible and rolls it back to the latest successful save", async () => {
    const user = userEvent.setup();
    const firstSave = deferred<{
      schemaVersion: number;
      area: "personal";
      voice: number;
      detail: number;
      initiative: number;
      humor: number;
    }>();
    const secondSave = deferred<never>();
    mocks.setClovyPersona
      .mockReturnValueOnce(firstSave.promise)
      .mockReturnValueOnce(secondSave.promise);
    render(<ClovyPersonalitySettingsSection />);
    await screen.findByRole("radio", { name: /Calm/ });

    await user.click(screen.getByRole("radio", { name: /Quick-witted/ }));
    await user.click(screen.getByRole("radio", { name: /Strategic/ }));

    await act(async () => {
      firstSave.resolve({
        schemaVersion: 1,
        area: "personal",
        voice: 85,
        detail: 70,
        initiative: 80,
        humor: 95,
      });
    });
    await waitFor(() => expect(mocks.setClovyPersona).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("radio", { name: /Strategic/ })).toBeChecked();

    await act(async () => {
      secondSave.reject(new Error("Could not save queued persona"));
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save queued persona");
    expect(screen.getByRole("radio", { name: /Quick-witted/ })).toBeChecked();
    expect(onboardingMood()).toBe("quick-witted");
  });

  it("keeps routine sounds and skills in Agent settings", async () => {
    render(<AgentSettingsSection />);
    await screen.findByText("No skills found.");
    expect(screen.getByRole("switch", { name: "Play agent sounds" })).toBeVisible();
    expect(screen.queryByRole("switch", { name: "Show sessions HUD" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Personality" })).toBeNull();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const skillFixture: AgentSkillDto = {
  id: "research",
  name: "Research",
  description: "Research instructions",
  source: "managed",
  enabled: true,
  configuredEnabled: true,
  readable: true,
  error: null,
  editable: true,
};
