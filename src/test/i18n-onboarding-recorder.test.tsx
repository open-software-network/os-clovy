import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DictationHistoryView } from "../components/dictation/DictationHistoryView";
import { OnboardingFlow } from "../components/onboarding/OnboardingFlow";
import { NoteRecoveryPrompt } from "../components/recorder/NoteRecoveryPrompt";
import { RecorderBar } from "../components/recorder/RecorderBar";
import { applyInterfaceLocale } from "../i18n/locale";
import type { RecordingStatusDto } from "../lib/tauri";

const mocks = vi.hoisted(() => ({
  dictationSettings: vi.fn(),
  dictationHelperCommand: vi.fn(),
  listDictationHistory: vi.fn(),
  listDictionaryEntries: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  dictationCapabilities: vi.fn().mockResolvedValue({
    capabilities: {
      available: true,
      platform: "macos",
      shortcuts: true,
      paste: true,
      microphoneSelection: true,
      accessibilityPermission: true,
      systemAudio: true,
    },
  }),
  dictationSettings: mocks.dictationSettings,
  dictationHelperCommand: mocks.dictationHelperCommand,
  checkRecordingSourceReadiness: vi.fn().mockResolvedValue({ sources: [] }),
  openPrivacySettings: vi.fn(),
  setDictationShortcut: vi.fn().mockResolvedValue(undefined),
  setClovyPersona: vi.fn(),
  listDictationHistory: mocks.listDictationHistory,
  listDictionaryEntries: mocks.listDictionaryEntries,
  deleteDictationHistoryItem: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen, emit: vi.fn() }));

const recordingStatus: RecordingStatusDto = {
  sessionId: "session-1",
  state: "recording",
  elapsedMs: 65_000,
  level: { peak: 0.2, rms: 0.1, recentPeaks: [0.2] },
  silenceWarning: false,
  bytesWritten: 4096,
};

beforeEach(() => {
  localStorage.clear();
  window.history.replaceState({}, "", "/");
  mocks.listen.mockResolvedValue(vi.fn());
  mocks.dictationHelperCommand.mockResolvedValue(undefined);
  mocks.dictationSettings.mockResolvedValue({
    settings: {
      pushToTalkShortcut: {
        code: "KeyD",
        label: "Ctrl+Opt+D",
        pressCount: 1,
        modifiers: { command: false, control: true, option: true, shift: false, function: false },
      },
      toggleShortcut: {
        code: "KeyT",
        label: "Ctrl+Opt+T",
        pressCount: 1,
        modifiers: { command: false, control: true, option: true, shift: false, function: false },
      },
      microphone: {},
      style: "standard",
    },
  });
  mocks.listDictionaryEntries.mockResolvedValue([]);
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

afterEach(() => {
  applyInterfaceLocale("en");
  localStorage.clear();
});

describe("onboarding i18n", () => {
  it("renders the permissions step in Portuguese", () => {
    applyInterfaceLocale("pt-BR");
    window.history.replaceState({}, "", "/?clovyDemoStep=permissions");
    render(<OnboardingFlow onComplete={vi.fn()} />);

    expect(
      screen.getByRole("heading", { name: "Deixe o Clovy ouvir e digitar" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continuar" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Voltar" })).toBeInTheDocument();
  });
});

describe("recorder i18n", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));

  it("labels the recorder controls in Portuguese", () => {
    render(
      <RecorderBar
        status={recordingStatus}
        onPause={vi.fn()}
        onResume={vi.fn()}
        onDone={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Pausar" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Concluído" })).toBeInTheDocument();
    expect(screen.getByLabelText("Atividade de áudio")).toBeInTheDocument();

    act(() => applyInterfaceLocale("en"));
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("formats the recovered size with a Portuguese decimal separator", () => {
    render(
      <NoteRecoveryPrompt
        recovery={{
          sessionId: "session-1",
          noteId: "note-1",
          startedAt: "2026-05-19T10:00:00Z",
          partialPathPresent: true,
          finalPathPresent: false,
          bytesFound: 1536,
        }}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
      />,
    );

    expect(
      screen.getByText(/Esta gravação foi interrompida\. Salvamos 1,5 KB de áudio\./),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Recuperar" })).toBeInTheDocument();
  });

  it("renders dictation history in Portuguese with plural retention copy", async () => {
    render(<DictationHistoryView />);

    expect(await screen.findByText("Send the follow up.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: /Ditado/ })).toBeInTheDocument();
    expect(screen.getByText("Transcrições por IA dos últimos 7 dias.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "Hoje" })).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Buscar")).toBeInTheDocument();
    expect(screen.getByText("Pressione para falar")).toBeInTheDocument();
  });
});
