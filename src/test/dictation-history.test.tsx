import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DictationHistoryView } from "../components/dictation/DictationHistoryView";

const mocks = vi.hoisted(() => ({
  listDictationHistory: vi.fn(),
  deleteDictationHistoryItem: vi.fn(),
  dictationSettings: vi.fn(),
  dictationCapabilities: vi.fn(),
  listDictionaryEntries: vi.fn(),
  listen: vi.fn(),
  writeText: vi.fn(),
  activityAppIcon: vi.fn(),
}));

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

vi.mock("../lib/tauri", () => ({
  listDictationHistory: mocks.listDictationHistory,
  deleteDictationHistoryItem: mocks.deleteDictationHistoryItem,
  dictationSettings: mocks.dictationSettings,
  dictationCapabilities: mocks.dictationCapabilities,
  listDictionaryEntries: mocks.listDictionaryEntries,
}));

vi.mock("../lib/activity-timeline", () => ({ activityAppIcon: mocks.activityAppIcon }));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

describe("DictationHistoryView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.listen.mockResolvedValue(vi.fn());
    mocks.activityAppIcon.mockResolvedValue(null);
    mocks.writeText.mockResolvedValue(undefined);
    mocks.deleteDictationHistoryItem.mockResolvedValue(undefined);
    mocks.listDictionaryEntries.mockResolvedValue([]);
    mocks.dictationCapabilities.mockResolvedValue({
      capabilities: {
        available: true,
        platform: "macos",
        shortcuts: true,
        paste: true,
        microphoneSelection: true,
        accessibilityPermission: true,
        systemAudio: true,
      },
    });
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          code: "KeyD",
          label: "Ctrl+Opt+D",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        toggleShortcut: {
          code: "KeyT",
          label: "Ctrl+Opt+T",
          pressCount: 1,
          modifiers: {
            command: false,
            control: true,
            option: true,
            shift: false,
            function: false,
          },
        },
        microphone: {},
        style: "standard",
      },
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: mocks.writeText },
    });
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: [
        {
          id: "dictation-1",
          text: "Send the follow up.",
          language: "en",
          provider: "openai",
          createdAt: new Date().toISOString(),
        },
      ],
    });
  });

  it("shows the target app icon and falls back when missing", async () => {
    mocks.activityAppIcon.mockImplementation(async (id: string) =>
      id === "app.found" ? "data:image/png;base64,AA==" : null,
    );
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: [
        {
          id: "found",
          text: "Found app",
          provider: "local",
          createdAt: new Date().toISOString(),
          appBundleId: "app.found",
        },
        {
          id: "missing",
          text: "Missing app",
          provider: "local",
          createdAt: new Date().toISOString(),
          appBundleId: "app.missing",
        },
        { id: "old", text: "Old item", provider: "local", createdAt: new Date().toISOString() },
      ],
    });
    render(<DictationHistoryView />);
    const found = (await screen.findByText("Found app")).closest("li");
    const missing = screen.getByText("Missing app").closest("li");
    const old = screen.getByText("Old item").closest("li");
    if (!found || !missing || !old) throw new Error("history rows missing");
    await waitFor(() =>
      expect(found.querySelector("img.app-icon")).toHaveAttribute(
        "src",
        "data:image/png;base64,AA==",
      ),
    );
    expect(within(missing).getByTestId("app-icon-fallback")).toBeInTheDocument();
    expect(within(old).getByTestId("app-icon-fallback")).toBeInTheDocument();
    const image = found.querySelector("img.app-icon");
    if (!image) throw new Error("app icon missing");
    fireEvent.error(image);
    expect(within(found).getByTestId("app-icon-fallback")).toBeInTheDocument();
  });

  it("renders recent dictations and copies with trailing space", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: mocks.writeText },
    });
    render(<DictationHistoryView />);

    await waitFor(() => expect(screen.getByText("Send the follow up.")).toBeInTheDocument());
    expect(screen.getByText("Push to talk")).toBeInTheDocument();
    expect(screen.getByText("Hands-free")).toBeInTheDocument();
    expect(screen.getByLabelText("Shortcut Ctrl+Opt+T")).toBeInTheDocument();

    const copyButton = screen.getByRole("button", { name: "Copy" });
    const iconSwap = copyButton.querySelector(".t-icon-swap");
    expect(iconSwap).toHaveAttribute("data-state", "a");
    expect(iconSwap?.querySelectorAll(".t-icon")).toHaveLength(2);
    expect(iconSwap?.querySelector('[data-icon="a"]')).not.toBeNull();
    expect(iconSwap?.querySelector('[data-icon="b"]')).not.toBeNull();

    copyButton.focus();
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Copy");

    vi.useFakeTimers();
    try {
      await act(async () => {
        fireEvent.click(copyButton);
        await Promise.resolve();
      });
      expect(mocks.writeText).toHaveBeenCalledWith("Send the follow up. ");
      expect(copyButton).toHaveAccessibleName("Copied");
      expect(iconSwap).toHaveAttribute("data-state", "b");
      expect(screen.getByRole("tooltip")).toHaveTextContent("Copied");

      act(() => vi.advanceTimersByTime(1200));
      await act(async () => {
        fireEvent.click(copyButton);
        await Promise.resolve();
      });
      expect(mocks.writeText).toHaveBeenCalledTimes(2);

      act(() => vi.advanceTimersByTime(401));
      expect(copyButton).toHaveAccessibleName("Copied");
      expect(iconSwap).toHaveAttribute("data-state", "b");

      act(() => vi.advanceTimersByTime(1199));
      expect(copyButton).toHaveAccessibleName("Copy");
      expect(iconSwap).toHaveAttribute("data-state", "a");
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps dialog copy feedback inside a fixed button footprint", async () => {
    mocks.activityAppIcon.mockResolvedValue("data:image/png;base64,AA==");
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: [
        {
          id: "dictation-1",
          text: "Send the follow up.",
          language: "en",
          provider: "local",
          createdAt: new Date().toISOString(),
          appBundleId: "app.editor",
        },
      ],
    });
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: mocks.writeText },
    });
    const scrollHeight = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(48);
    const clientHeight = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(24);

    try {
      render(<DictationHistoryView />);

      const transcript = await screen.findByRole("button", { name: "Show full transcript" });
      const row = transcript.closest("li");
      expect(row).not.toBeNull();
      const rowCopyButton = within(row as HTMLElement).getByRole("button", { name: "Copy" });
      const rowIconSwap = rowCopyButton.querySelector(".t-icon-swap");

      await user.click(transcript);
      const dialog = screen.getByRole("dialog");
      await waitFor(() =>
        expect(dialog.querySelector("img.app-icon")).toHaveAttribute(
          "src",
          "data:image/png;base64,AA==",
        ),
      );
      const dialogCopyButton = within(dialog).getByRole("button", { name: "Copy" });
      const dialogIconSwap = dialogCopyButton.querySelector(".t-icon-swap");

      expect(dialogCopyButton).toHaveTextContent(/^Copy$/);
      expect(dialogIconSwap).toHaveAttribute("data-state", "a");
      expect(dialogIconSwap?.querySelectorAll(".t-icon")).toHaveLength(2);
      expect(dialogIconSwap?.querySelector('[data-icon="a"]')).not.toBeNull();
      expect(dialogIconSwap?.querySelector('[data-icon="b"]')).not.toBeNull();

      dialogCopyButton.focus();
      expect(await screen.findByRole("tooltip")).toHaveTextContent("Copy");

      await user.click(dialogCopyButton);
      await waitFor(() => expect(mocks.writeText).toHaveBeenCalledWith("Send the follow up. "));
      expect(dialogCopyButton).toHaveAccessibleName("Copied");
      expect(dialogCopyButton).toHaveTextContent(/^Copy$/);
      expect(dialogIconSwap).toHaveAttribute("data-state", "b");
      expect(rowIconSwap).toHaveAttribute("data-state", "b");
      expect(screen.getByRole("tooltip")).toHaveTextContent("Copied");
    } finally {
      scrollHeight.mockRestore();
      clientHeight.mockRestore();
    }
  });

  it("advertises shortcut dictation on Windows", async () => {
    const restoreNavigator = stubNavigatorPlatform(
      "Win32",
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    );
    try {
      mocks.dictationCapabilities.mockResolvedValue({
        capabilities: {
          available: true,
          platform: "windows",
          shortcuts: true,
          paste: true,
          microphoneSelection: true,
          accessibilityPermission: false,
          systemAudio: false,
        },
      });
      mocks.dictationSettings.mockResolvedValue({
        settings: {
          pushToTalkShortcut: {
            code: "KeyD",
            label: "Ctrl+Alt+D",
            pressCount: 1,
            modifiers: {
              command: false,
              control: true,
              option: true,
              shift: false,
              function: false,
            },
          },
          toggleShortcut: {
            code: "KeyT",
            label: "Ctrl+Alt+T",
            pressCount: 1,
            modifiers: {
              command: false,
              control: true,
              option: true,
              shift: false,
              function: false,
            },
          },
          microphone: {},
          style: "standard",
        },
      });
      mocks.listDictationHistory.mockResolvedValue({
        retentionDays: 7,
        items: [],
      });

      render(<DictationHistoryView />);

      expect(await screen.findByText("Start dictating anywhere")).toBeInTheDocument();
      expect(
        screen.getByText(
          "Place your cursor in any app, hold the shortcut, and speak. Your words are transcribed and pasted right where you're typing.",
        ),
      ).toBeInTheDocument();
      expect(screen.getByLabelText("Shortcut Ctrl+Alt+D")).toBeInTheDocument();
      expect(screen.getByLabelText("Shortcut Ctrl+Alt+T")).toBeInTheDocument();
    } finally {
      restoreNavigator();
    }
  });

  function manyItems(count: number) {
    return Array.from({ length: count }, (_, i) => ({
      id: `dictation-${i}`,
      text: `Transcript ${i}`,
      language: "en",
      provider: "openai",
      createdAt: new Date().toISOString(),
    }));
  }

  it("hides the 'Get more' card until dictation is adopted", async () => {
    // Only one recent dictation — below the adoption threshold.
    render(<DictationHistoryView />);
    await waitFor(() => expect(screen.getByText("Send the follow up.")).toBeInTheDocument());
    expect(screen.queryByText("Get more from dictation")).not.toBeInTheDocument();
  });

  it("surfaces only the unconfigured features once adopted", async () => {
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: manyItems(12),
    });
    // Style is configured, dictionary is not → only dictionary should show.
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          code: "KeyD",
          label: "Ctrl+Opt+D",
          pressCount: 1,
          modifiers: {},
        },
        toggleShortcut: {
          code: "KeyT",
          label: "Ctrl+Opt+T",
          pressCount: 1,
          modifiers: {},
        },
        microphone: {},
        style: "formal",
      },
    });
    mocks.listDictionaryEntries.mockResolvedValue([]);

    render(<DictationHistoryView />);
    await waitFor(() => expect(screen.getByText("Get more from dictation")).toBeInTheDocument());
    expect(screen.getByText("Personal dictionary")).toBeInTheDocument();
    expect(screen.queryByText("Writing style")).not.toBeInTheDocument();
  });

  it("never shows the card once both features are configured", async () => {
    mocks.listDictationHistory.mockResolvedValue({
      retentionDays: 7,
      items: manyItems(12),
    });
    mocks.dictationSettings.mockResolvedValue({
      settings: {
        pushToTalkShortcut: {
          code: "KeyD",
          label: "Ctrl+Opt+D",
          pressCount: 1,
          modifiers: {},
        },
        toggleShortcut: {
          code: "KeyT",
          label: "Ctrl+Opt+T",
          pressCount: 1,
          modifiers: {},
        },
        microphone: {},
        style: "casualLowercase",
      },
    });
    mocks.listDictionaryEntries.mockResolvedValue([{ id: "e1", phrase: "Bismarck" }]);

    render(<DictationHistoryView />);
    await waitFor(() => expect(screen.getByText("Transcript 0")).toBeInTheDocument());
    expect(screen.queryByText("Get more from dictation")).not.toBeInTheDocument();
  });

  it("deletes a transcription after confirmation", async () => {
    const user = userEvent.setup();
    render(<DictationHistoryView />);

    await waitFor(() => expect(screen.getByText("Send the follow up.")).toBeInTheDocument());

    // Row trash icon opens the confirmation dialog rather than deleting.
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText("Delete this transcription?")).toBeInTheDocument();
    expect(mocks.deleteDictationHistoryItem).not.toHaveBeenCalled();

    // Confirm: the dialog's destructive button is the last "Delete".
    const deleteButtons = screen.getAllByRole("button", { name: "Delete" });
    await user.click(deleteButtons[deleteButtons.length - 1]);

    await waitFor(() =>
      expect(mocks.deleteDictationHistoryItem).toHaveBeenCalledWith("dictation-1"),
    );
    expect(screen.queryByText("Send the follow up.")).not.toBeInTheDocument();
  });
});
