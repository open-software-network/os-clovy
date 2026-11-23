import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LlmProvidersSection } from "../components/settings/LlmProvidersSection";
import { applyInterfaceLocale } from "../i18n/locale";
import type { LlmCliStatusDto, LlmProvidersDto } from "../lib/llm-providers";
import { dispatchProviderModelSettingsChanged } from "../lib/model-privacy";

const mocks = vi.hoisted(() => ({
  llmProviders: vi.fn(),
  llmDetectClis: vi.fn(),
  llmSaveEndpoint: vi.fn(),
  llmDeleteEndpoint: vi.fn(),
  llmTestProvider: vi.fn(),
  llmSetUsage: vi.fn(),
  openAntigravityLogin: vi.fn(),
  probeLocalGenerationEndpoint: vi.fn(),
}));

vi.mock("../lib/llm-providers", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/llm-providers")>()),
  llmProviders: mocks.llmProviders,
  llmDetectClis: mocks.llmDetectClis,
  llmSaveEndpoint: mocks.llmSaveEndpoint,
  llmDeleteEndpoint: mocks.llmDeleteEndpoint,
  llmTestProvider: mocks.llmTestProvider,
  llmSetUsage: mocks.llmSetUsage,
  openAntigravityLogin: mocks.openAntigravityLogin,
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  probeLocalGenerationEndpoint: mocks.probeLocalGenerationEndpoint,
}));

const sampleClis: LlmCliStatusDto[] = [
  {
    id: "claude",
    name: "Claude Code",
    installed: true,
    path: "/usr/local/bin/claude",
    version: "1.0.0",
    structuredOutput: "strict",
    oneShot: true,
  },
  {
    id: "codex",
    name: "Codex",
    installed: false,
    reason: "`codex` was not found on your PATH.",
    structuredOutput: "none",
    oneShot: true,
  },
  {
    id: "pi",
    name: "Pi",
    installed: false,
    structuredOutput: "prompt",
    oneShot: true,
  },
  {
    id: "agy",
    name: "Antigravity",
    installed: false,
    structuredOutput: "none",
    oneShot: true,
  },
  {
    id: "cursor-agent",
    name: "Cursor Agent",
    installed: false,
    structuredOutput: "none",
    oneShot: false,
  },
  {
    id: "copilot",
    name: "GitHub Copilot",
    installed: false,
    structuredOutput: "none",
    oneShot: false,
  },
];

const sampleProviders: LlmProvidersDto = {
  endpoints: [
    {
      id: "ep-1",
      name: "Local Ollama",
      baseUrl: "http://localhost:11434/v1",
      modelId: "llama3.2:3b",
      hasApiKey: true,
      structuredOutput: "json_schema",
    },
  ],
  usage: {
    chat: { kind: "cli", id: "claude" },
    notes: { kind: "openrouter" },
    activity: { kind: "none" },
  },
  cliLevels: {},
};

describe("LlmProvidersSection", () => {
  it.each([
    ["en", "Authenticate in Clovy", "Opening terminal..."],
    ["pt-BR", "Autenticar no Clovy", "Abrindo terminal..."],
  ] as const)("managed profile login handles keyboard, pending, failure and retry in %s", async (locale, label, opening) => {
    applyInterfaceLocale(locale);
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) => (cli.id === "agy" ? { ...cli, installed: true } : cli)),
    );
    let reject: (error: Error) => void = () => undefined;
    mocks.openAntigravityLogin.mockImplementationOnce(
      () =>
        new Promise<void>((_resolve, fail) => {
          reject = fail;
        }),
    );
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    const action = await screen.findByRole("button", { name: label });
    action.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: opening })).toBeDisabled();
    await user.keyboard("{Enter}");
    expect(mocks.openAntigravityLogin).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error("Synthetic terminal unavailable")));
    expect(await screen.findByText(/Synthetic terminal unavailable/)).toBeInTheDocument();
    mocks.openAntigravityLogin.mockResolvedValueOnce(undefined);
    await user.click(screen.getByRole("button", { name: label }));
    await waitFor(() => expect(screen.getByRole("button", { name: label })).toBeEnabled());
    expect(screen.queryByText(/Synthetic terminal unavailable/)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/successfully authenticated|autenticado com sucesso/i),
    ).not.toBeInTheDocument();
  });

  it("does not offer managed profile authentication without an installed Antigravity", async () => {
    render(<LlmProvidersSection />);
    await screen.findByRole("heading", { name: "Antigravity" });
    expect(screen.queryByRole("button", { name: "Authenticate in Clovy" })).not.toBeInTheDocument();
    expect(mocks.openAntigravityLogin).not.toHaveBeenCalled();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.llmProviders.mockResolvedValue(sampleProviders);
    mocks.llmDetectClis.mockResolvedValue(sampleClis);
    mocks.llmSaveEndpoint.mockImplementation(async (req) => ({
      ...sampleProviders,
      endpoints: [
        ...sampleProviders.endpoints,
        {
          id: req.id ?? "ep-new",
          name: req.name,
          baseUrl: req.baseUrl,
          modelId: req.modelId,
          hasApiKey: Boolean(req.apiKey),
        },
      ],
    }));
    mocks.llmDeleteEndpoint.mockResolvedValue(sampleProviders);
    mocks.llmTestProvider.mockResolvedValue({
      latencyMs: 142,
      structuredOutput: "strict",
    });
    mocks.llmSetUsage.mockImplementation(async (usage, provider) => ({
      ...sampleProviders,
      usage: {
        ...sampleProviders.usage,
        [usage]: provider,
      },
    }));
  });

  afterEach(() => {
    cleanup();
    applyInterfaceLocale("en");
  });

  it("renders installed vs not-installed CLIs with path/version vs reason, and disables not-installed in selects", async () => {
    render(<LlmProvidersSection />);

    expect(await screen.findByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    expect(screen.getByText(/\/usr\/local\/bin\/claude · 1.0.0/)).toBeInTheDocument();

    expect(screen.getByRole("heading", { name: "Codex" })).toBeInTheDocument();
    // Non-installed CLI reasons
    expect(screen.getAllByText("Not found on your PATH.").length).toBeGreaterThan(0);
    // Usage select checks
    const chatSelect = screen.getByRole("combobox", { name: "Default agent CLI" });
    const installedOption = chatSelect.querySelector(
      'option[value="cli:claude"]',
    ) as HTMLOptionElement;
    const notInstalledOption = chatSelect.querySelector(
      'option[value="cli:codex"]',
    ) as HTMLOptionElement;

    expect(installedOption).toBeInTheDocument();
    expect(installedOption.disabled).toBe(false);
    expect(installedOption.textContent).toBe("Claude Code");

    expect(notInstalledOption).toBeInTheDocument();
    expect(notInstalledOption.disabled).toBe(true);
    expect(notInstalledOption.textContent).toContain("(not installed)");
  });

  it("offers codex and agy for content uses in their least-privileged mode", async () => {
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) =>
        cli.id === "codex" || cli.id === "agy"
          ? { ...cli, installed: true, path: `/usr/local/bin/${cli.id}`, oneShot: true }
          : cli,
      ),
    );
    render(<LlmProvidersSection />);

    expect(await screen.findByText(/\/usr\/local\/bin\/codex/)).toBeInTheDocument();
    const option = (name: string, id: string) =>
      screen
        .getByRole("combobox", { name })
        .querySelector(`option[value="cli:${id}"]`) as HTMLOptionElement;
    for (const id of ["codex", "agy"]) {
      for (const name of ["Default agent CLI", "Notes", "Activity"]) {
        expect(option(name, id).disabled, `${id} ${name}`).toBe(false);
      }
    }
    expect(option("Notes", "codex").textContent).toBe("Codex");
    // Claude Code, Codex, and Antigravity are installed and can be tested.
    expect(screen.getAllByRole("button", { name: "Test" })).toHaveLength(3);
  });

  it("offers a CLI whose tools stay on for the default agent, but not for notes or activity, and does not test it", async () => {
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) =>
        cli.id === "cursor-agent"
          ? { ...cli, installed: true, path: "/usr/local/bin/cursor-agent" }
          : cli,
      ),
    );
    render(<LlmProvidersSection />);

    expect(await screen.findByText(/\/usr\/local\/bin\/cursor-agent/)).toBeInTheDocument();
    const option = (name: string) =>
      screen
        .getByRole("combobox", { name })
        .querySelector('option[value="cli:cursor-agent"]') as HTMLOptionElement;
    expect(option("Default agent CLI").disabled).toBe(false);
    // Dictation cleanup never runs on a CLI or endpoint, so it has no row.
    expect(screen.queryByRole("combobox", { name: "Dictation cleanup" })).toBeNull();
    for (const name of ["Notes", "Activity"]) {
      expect(option(name).disabled, name).toBe(true);
      expect(option(name).textContent).toBe("Cursor Agent (tools can't be turned off)");
    }
    expect(screen.getAllByText(/This CLI keeps its tools enabled/).length).toBe(2);
    // Only Claude Code (installed, one-shot capable) has a test button.
    expect(screen.getAllByRole("button", { name: "Test" })).toHaveLength(1);
  });

  it("test button shows latency and level", async () => {
    const user = userEvent.setup();
    render(<LlmProvidersSection />);

    expect(await screen.findByRole("heading", { name: "Claude Code" })).toBeInTheDocument();
    const testButton = screen.getByRole("button", { name: "Test" });
    await user.click(testButton);

    expect(mocks.llmTestProvider).toHaveBeenCalledWith({ kind: "cli", id: "claude" });
    expect(
      await screen.findByText(/Connected in 142 ms · Structured output: Strict/),
    ).toBeInTheDocument();
  });

  it("add endpoint sends request without echoing key back and shows 'API key saved' from hasApiKey", async () => {
    const user = userEvent.setup();
    render(<LlmProvidersSection />);

    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();
    expect(screen.getByText(/API key saved/)).toBeInTheDocument();

    // Verify the secret key is never rendered in HTML
    expect(screen.queryByText("supersecretkey")).not.toBeInTheDocument();

    const addBtn = screen.getByRole("button", { name: "Add endpoint" });
    await user.click(addBtn);

    expect(screen.getByRole("dialog")).toBeInTheDocument();

    const nameInput = screen.getByLabelText("Name");
    const urlInput = screen.getByLabelText("Base URL");
    const modelInput = screen.getByLabelText("Model ID");
    const keyInput = screen.getByLabelText("API key");

    await user.type(nameInput, "Custom Gateway");
    await user.type(urlInput, "http://localhost:8000/v1");
    await user.type(modelInput, "custom-model");
    await user.type(keyInput, "my-secret-token");

    const saveBtn = screen.getByRole("button", { name: "Save endpoint" });
    await user.click(saveBtn);

    expect(mocks.llmSaveEndpoint).toHaveBeenCalledWith({
      id: undefined,
      name: "Custom Gateway",
      baseUrl: "http://localhost:8000/v1",
      modelId: "custom-model",
      apiKey: "my-secret-token",
      clearApiKey: undefined,
    });
  });

  it("choosing activity provider rejected with llm_structured_output_insufficient shows message and keeps previous value", async () => {
    const user = userEvent.setup();
    mocks.llmSetUsage.mockRejectedValue({
      code: "llm_structured_output_insufficient",
      // The native message (src-tauri/src/llm/mod.rs), worded by code in the UI.
      message:
        "Activity features need a provider with JSON schema support. Test the provider first, or choose one that supports JSON schema.",
    });

    render(<LlmProvidersSection />);

    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();

    const activitySelect = screen.getByRole("combobox", { name: "Activity" }) as HTMLSelectElement;
    expect(activitySelect.value).toBe("none");

    await user.selectOptions(activitySelect, "endpoint:ep-1");

    expect(
      await screen.findByText(
        "Activity features need a provider with JSON schema support. Test the provider first, or choose one that supports JSON schema.",
      ),
    ).toBeInTheDocument();
    // Select keeps the previous value
    expect(activitySelect.value).toBe("none");
  });

  it("tests an unmeasured Activity CLI before applying it, showing pending feedback", async () => {
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) => (cli.id === "agy" ? { ...cli, installed: true } : cli)),
    );
    let finish: (value: { latencyMs: number; structuredOutput: "json_schema" }) => void = () =>
      undefined;
    mocks.llmTestProvider.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const user = userEvent.setup();
    const onProvidersChanged = vi.fn();
    render(<LlmProvidersSection onProvidersChanged={onProvidersChanged} />);
    const select = await screen.findByRole("combobox", { name: "Activity" });
    await user.selectOptions(select, "cli:agy");
    expect(mocks.llmTestProvider).toHaveBeenCalledWith({ kind: "cli", id: "agy" });
    expect(mocks.llmSetUsage).not.toHaveBeenCalled();
    expect(select).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("Testing...");
    expect(screen.getByRole("button", { name: "Test Local Ollama" })).toBeDisabled();
    for (const button of screen.getAllByRole("button", { name: /^Test(?:ing\.\.\.)?$/ })) {
      expect(button).toBeDisabled();
    }
    await act(async () => finish({ latencyMs: 42, structuredOutput: "json_schema" }));
    await waitFor(() => expect(select).toHaveValue("cli:agy"));
    expect(mocks.llmSetUsage).toHaveBeenCalledWith("activity", { kind: "cli", id: "agy" });
    expect(select).toBeEnabled();
    expect(onProvidersChanged).toHaveBeenLastCalledWith(
      expect.objectContaining({
        usage: expect.objectContaining({ activity: { kind: "cli", id: "agy" } }),
      }),
    );
    expect(mocks.openAntigravityLogin).not.toHaveBeenCalled();
  });

  it.each([
    [
      "llm_cli_failed",
      "Antigravity failed: Not authenticated. Please login.",
      "Use Authenticate in Clovy",
    ],
    ["llm_cli_failed", "Antigravity failed: Authentication failed", "Use Authenticate in Clovy"],
    ["llm_cli_failed", "Antigravity failed: Please log in", "Use Authenticate in Clovy"],
    ["llm_cli_failed", "Antigravity failed: HTTP 401 Unauthorized", "Use Authenticate in Clovy"],
    [
      "llm_cli_failed",
      "Antigravity failed: quota exhausted; please login again",
      "provider's usage limit",
    ],
    [
      "llm_cli_failed",
      "Antigravity failed: synthetic executable unavailable",
      "synthetic executable unavailable",
    ],
    ["llm_cli_failed", "Antigravity failed: quota exhausted", "provider's usage limit"],
    ["llm_timeout", "The provider timed out.", "did not answer in time"],
  ])("keeps Activity unchanged on a failed automatic test (%s: %s)", async (code, message, expected) => {
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) => (cli.id === "agy" ? { ...cli, installed: true } : cli)),
    );
    mocks.llmTestProvider.mockRejectedValueOnce({ code, message });
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    const select = await screen.findByRole("combobox", { name: "Activity" });
    await user.selectOptions(select, "cli:agy");
    expect(await screen.findByRole("alert")).toHaveTextContent(expected);
    expect(screen.getByRole("alert")).not.toHaveTextContent("JSON schema");
    expect(select).toHaveValue("none");
    expect(select).toBeEnabled();
    expect(mocks.llmSetUsage).not.toHaveBeenCalled();
    expect(mocks.openAntigravityLogin).not.toHaveBeenCalled();
  });

  it("explains missing managed-profile login in Portuguese without opening authentication", async () => {
    applyInterfaceLocale("pt-BR");
    mocks.llmDetectClis.mockResolvedValue(
      sampleClis.map((cli) => (cli.id === "agy" ? { ...cli, installed: true } : cli)),
    );
    mocks.llmTestProvider.mockRejectedValueOnce({
      code: "llm_cli_failed",
      message: "Antigravity failed: not logged in",
    });
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    await user.selectOptions(await screen.findByRole("combobox", { name: "Atividade" }), "cli:agy");
    expect(await screen.findByRole("alert")).toHaveTextContent("Use Autenticar no Clovy");
    expect(mocks.openAntigravityLogin).not.toHaveBeenCalled();
  });

  it("keeps an insufficient measured result and does not retest it on another choice", async () => {
    mocks.llmTestProvider.mockResolvedValueOnce({ latencyMs: 42, structuredOutput: "none" });
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    const select = await screen.findByRole("combobox", { name: "Activity" });
    await user.selectOptions(select, "cli:claude");
    expect(await screen.findByRole("alert")).toHaveTextContent("JSON schema support");
    expect(select).toHaveValue("none");
    expect(mocks.llmSetUsage).not.toHaveBeenCalled();
    mocks.llmSetUsage.mockRejectedValueOnce({
      code: "llm_structured_output_insufficient",
      message: "Insufficient",
    });
    await user.selectOptions(select, "cli:claude");
    await waitFor(() => expect(select).toBeEnabled());
    expect(mocks.llmTestProvider).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole("alert")).toHaveTextContent("JSON schema support");
  });

  it("does not automatically test an already measured CLI or a Notes choice", async () => {
    mocks.llmProviders.mockResolvedValue({ ...sampleProviders, cliLevels: { claude: "strict" } });
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    const select = await screen.findByRole("combobox", { name: "Activity" });
    await user.selectOptions(select, "cli:claude");
    await waitFor(() => expect(select).toHaveValue("cli:claude"));
    await user.selectOptions(screen.getByRole("combobox", { name: "Notes" }), "cli:claude");
    expect(mocks.llmTestProvider).not.toHaveBeenCalled();
  });

  it("default activity selection renders as None", async () => {
    render(<LlmProvidersSection />);
    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();

    const activitySelect = screen.getByRole("combobox", { name: "Activity" }) as HTMLSelectElement;
    expect(activitySelect.value).toBe("none");
    expect(
      screen.getByText("Activity stays off when set to None. Nothing is sent anywhere."),
    ).toBeInTheDocument();
  });

  it("chat set to a CLI updates the selection without the later-update note", async () => {
    const user = userEvent.setup();
    render(<LlmProvidersSection />);
    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();

    const chatSelect = screen.getByRole("combobox", { name: "Default agent CLI" });
    await user.selectOptions(chatSelect, "cli:claude");

    expect(
      screen.queryByText(
        "Chat with agent CLIs arrives in a later update. Chat keeps using Clovy until then.",
      ),
    ).not.toBeInTheDocument();
  });

  it("lists only agent CLIs for chat while preserving note providers", async () => {
    render(<LlmProvidersSection />);
    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();

    const chatSelect = screen.getByRole("combobox", {
      name: "Default agent CLI",
    }) as HTMLSelectElement;
    const values = Array.from(chatSelect.options).map((option) => option.value);
    expect(values).toEqual([
      "cli:claude",
      "cli:codex",
      "cli:pi",
      "cli:agy",
      "cli:cursor-agent",
      "cli:copilot",
    ]);
    expect(values.some((value) => value.startsWith("endpoint:") || value === "clovy")).toBe(false);
    const notesSelect = screen.getByRole("combobox", { name: "Notes" }) as HTMLSelectElement;
    expect(notesSelect.options[0]).toHaveValue("openrouter");
    expect(notesSelect.options[0]).toHaveTextContent(/^OpenRouter$/);
  });

  it("does not offer the native fallback when no default CLI is installed", async () => {
    mocks.llmProviders.mockResolvedValue(sampleProviders);
    mocks.llmDetectClis.mockResolvedValue(sampleClis.map((cli) => ({ ...cli, installed: false })));
    render(<LlmProvidersSection />);
    const chatSelect = await screen.findByRole("combobox", { name: "Default agent CLI" });
    expect(chatSelect).toHaveValue("cli:claude");
    expect(screen.queryByText(/legacy Clovy agent/)).not.toBeInTheDocument();
  });

  it("says nothing about the legacy agent once chat is on the default CLI", async () => {
    mocks.llmProviders.mockResolvedValue({
      ...sampleProviders,
      usage: { ...sampleProviders.usage, chat: { kind: "cli", id: "claude" } },
    });
    render(<LlmProvidersSection />);
    expect(await screen.findByRole("heading", { name: "Local Ollama" })).toBeInTheDocument();
    expect(screen.queryByText(/legacy Clovy agent/)).not.toBeInTheDocument();
  });

  it("keeps chat on Claude Code when the text model changes elsewhere", async () => {
    mocks.llmProviders.mockResolvedValueOnce({
      ...sampleProviders,
      usage: { ...sampleProviders.usage, chat: { kind: "cli", id: "claude" } },
    });
    render(<LlmProvidersSection />);
    const chatSelect = (await screen.findByRole("combobox", {
      name: "Default agent CLI",
    })) as HTMLSelectElement;
    await waitFor(() => expect(chatSelect.value).toBe("cli:claude"));

    // Text models belong to notes; a new model must not move the chat engine.
    dispatchProviderModelSettingsChanged({ mode: "generation", modelId: "zai-org-glm-5-2" });

    await waitFor(() => expect(mocks.llmProviders).toHaveBeenCalledTimes(2));
    expect(chatSelect.value).toBe("cli:claude");
  });

  it("loads models of a saved endpoint with its stored key when no key is typed", async () => {
    const user = userEvent.setup();
    mocks.probeLocalGenerationEndpoint.mockResolvedValue({ models: ["llama3.2:3b"] });
    render(<LlmProvidersSection />);

    await user.click(await screen.findByRole("button", { name: "Configure Local Ollama" }));
    await user.click(screen.getByRole("button", { name: "Load models" }));

    expect(mocks.probeLocalGenerationEndpoint).toHaveBeenCalledWith({
      baseUrl: "http://localhost:11434/v1",
      apiKey: "",
      endpointId: "ep-1",
    });

    await user.type(screen.getByLabelText("API key"), "sk-new-key");
    await user.click(screen.getByRole("button", { name: "Load models" }));
    expect(mocks.probeLocalGenerationEndpoint).toHaveBeenLastCalledWith({
      baseUrl: "http://localhost:11434/v1",
      apiKey: "sk-new-key",
      endpointId: "ep-1",
    });
  });

  it("renders properly in pt-BR", async () => {
    applyInterfaceLocale("pt-BR");
    render(<LlmProvidersSection />);

    expect(await screen.findByText("Provedores de LLM")).toBeInTheDocument();
    expect(screen.getByText("Provedores para cada uso")).toBeInTheDocument();
    expect(screen.getByText("CLIs de agentes")).toBeInTheDocument();
    expect(screen.getByText("Endpoints")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Adicionar endpoint" })).toBeInTheDocument();
  });
});
