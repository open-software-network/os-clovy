import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  NoteFailureBanner,
  userFacingFailureMessage,
} from "../components/note-editor/NoteFailureBanner";
import { applyInterfaceLocale } from "../i18n/locale";

describe("userFacingFailureMessage", () => {
  it("turns no-speech provider codes into useful guidance", () => {
    expect(userFacingFailureMessage("Microphone: upstream_provider_failed; no_speech")).toBe(
      "Microphone: No speech detected. Try speaking louder or moving closer to the microphone.",
    );
  });

  it("hides raw JSON parser failures from saved notes", () => {
    expect(userFacingFailureMessage("expected value at line 1 column 1")).toBe(
      "The processing service returned an invalid response.",
    );
    expect(userFacingFailureMessage("Microphone: expected value at line 1 column 1")).toBe(
      "Microphone: The processing service returned an invalid response.",
    );
  });

  it("turns authorization denials into retry guidance", () => {
    expect(userFacingFailureMessage("authorization_denied")).toBe(
      "The service is busy right now. Wait a minute, then retry.",
    );
    expect(userFacingFailureMessage("Microphone: authorization_denied")).toBe(
      "Microphone: The service is busy right now. Wait a minute, then retry.",
    );
  });
});

describe("missing provider keys", () => {
  afterEach(() => applyInterfaceLocale("en"));

  it("explains an engine switch without blaming the local model", () => {
    applyInterfaceLocale("pt-BR");
    expect(
      userFacingFailureMessage(
        "Microphone: The transcription engine changed during processing. Retry to transcribe with the current engine.",
      ),
    ).toBe(
      "Microfone: O motor de transcrição mudou durante o processamento. Tente de novo para transcrever com o motor atual.",
    );
  });

  const groqMissing =
    "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings.";
  const openRouterMissing =
    "Your recording was transcribed, but Clovy needs your OpenRouter API key to write the note. Add it in Settings > Models > More options > Provider API keys. Your transcript is saved.";

  it("names the missing key per source instead of the raw request error", () => {
    expect(userFacingFailureMessage(`System: ${groqMissing} | Microphone: ${groqMissing}`)).toBe(
      `System: ${groqMissing} | Microphone: ${groqMissing}`,
    );
  });

  it("shows the missing keys in Portuguese", () => {
    applyInterfaceLocale("pt-BR");
    expect(userFacingFailureMessage(`Microphone: ${groqMissing}`)).toBe(
      "Microfone: Adicione sua chave de API da Groq em Configurações > Modelos > Mais opções > Chaves de API de provedores para transcrever gravações.",
    );
    expect(userFacingFailureMessage(openRouterMissing)).toBe(
      "Sua gravação foi transcrita, mas o Clovy precisa da sua chave de API da OpenRouter para escrever a nota. Adicione-a em Configurações > Modelos > Mais opções > Chaves de API de provedores. Sua transcrição está salva.",
    );
  });

  it("keeps Retry for a missing key", () => {
    render(
      <NoteFailureBanner errorMessage={groqMissing} audioPreserved onRetry={() => undefined} />,
    );
    expect(screen.getByRole("button", { name: /Retry/i })).toBeEnabled();
  });
});

describe("Groq rate limits", () => {
  afterEach(() => applyInterfaceLocale("en"));

  const usageLimit =
    "Groq reports that its usage limit was reached. Try again later, or check the plan behind your saved Groq key.";
  const paused =
    "Groq usage limit reached. Clovy continues transcribing at 14:35. Parts already transcribed are kept.";

  it("keeps the English sentences as stored", () => {
    expect(userFacingFailureMessage(`System: ${usageLimit} | Microphone: ${usageLimit}`)).toBe(
      `System: ${usageLimit} | Microphone: ${usageLimit}`,
    );
    expect(userFacingFailureMessage(paused)).toBe(paused);
  });

  it("shows the usage limit in Portuguese", () => {
    applyInterfaceLocale("pt-BR");
    expect(userFacingFailureMessage(`System: ${usageLimit} | Microphone: ${usageLimit}`)).toBe(
      "Sistema: A Groq informa que o limite de uso foi atingido. Tente de novo mais tarde ou confira o plano da sua chave Groq salva. | Microfone: A Groq informa que o limite de uso foi atingido. Tente de novo mais tarde ou confira o plano da sua chave Groq salva.",
    );
  });

  it("says in Portuguese that the saved Groq key could not be read", () => {
    applyInterfaceLocale("pt-BR");
    expect(
      userFacingFailureMessage(
        "Microphone: Clovy could not read your saved Groq key. Open Settings and save it again.",
      ),
    ).toBe(
      "Microfone: O Clovy não conseguiu ler sua chave Groq salva. Abra as Configurações e salve-a de novo.",
    );
  });

  it("shows when a paused note continues, in Portuguese", () => {
    applyInterfaceLocale("pt-BR");
    expect(userFacingFailureMessage(paused)).toBe(
      "Limite de uso da Groq atingido. O Clovy continua a transcrição às 14:35. As partes já transcritas ficam salvas.",
    );
  });
});

describe("NoteFailureBanner", () => {
  it("shows only Retry for generic failures and reassures audio is saved", () => {
    render(
      <NoteFailureBanner
        errorMessage="Network unreachable"
        audioPreserved
        onRetry={() => undefined}
      />,
    );
    expect(screen.getByText(/Network unreachable/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Upgrade/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Retry/i })).toBeEnabled();
    expect(screen.getByText(/Your recording is saved locally/i)).toBeInTheDocument();
  });

  it("shows a friendly message for no-speech transcription failures", () => {
    render(
      <NoteFailureBanner
        errorMessage="Microphone: upstream_provider_failed; no_speech"
        audioPreserved
        onRetry={() => undefined}
      />,
    );

    expect(screen.getByText(/No speech detected/i)).toBeInTheDocument();
    expect(screen.queryByText(/upstream_provider_failed/i)).not.toBeInTheDocument();
  });

  it("shows a neutral retry message for historical metering failures", () => {
    render(
      <NoteFailureBanner
        errorMessage="Microphone: metering_provider_failed"
        audioPreserved
        onRetry={() => undefined}
      />,
    );

    expect(screen.getByText(/transcription provider could not process/i)).toBeInTheDocument();
    expect(screen.queryByText(/metering_provider_failed/i)).not.toBeInTheDocument();
  });

  it("guards against double-click while a retry is in flight", async () => {
    let resolveRetry: () => void = () => {};
    const onRetry = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveRetry = resolve;
        }),
    );
    render(
      <NoteFailureBanner errorMessage="Network unreachable" audioPreserved onRetry={onRetry} />,
    );

    const retryButton = screen.getByRole("button", { name: /Retry/i });
    await userEvent.click(retryButton);

    // Button disables while the retry is in flight; aria-busy reflects it.
    expect(screen.getByRole("button", { name: /Retry/i })).toBeDisabled();
    expect(onRetry).toHaveBeenCalledTimes(1);

    // A second click while pending must not fire onRetry again.
    await userEvent.click(screen.getByRole("button", { name: /Retry/i }));
    expect(onRetry).toHaveBeenCalledTimes(1);

    // Resolve so the test doesn't hang on cleanup.
    resolveRetry();
  });

  it("disables Retry when no audio is preserved (e.g., recording itself failed)", () => {
    render(
      <NoteFailureBanner
        errorMessage="Recording sources not ready"
        audioPreserved={false}
        onRetry={() => undefined}
      />,
    );
    expect(screen.getByRole("button", { name: /Retry/i })).toBeDisabled();
  });
});

it("translates the stored local transcription failure and keeps retry", () => {
  applyInterfaceLocale("pt-BR");
  try {
    const message =
      "The local transcription model failed. Try again or reinstall it in Settings > Models.";
    expect(userFacingFailureMessage(message)).toBe(
      "O modelo local de transcrição falhou. Tente de novo ou reinstale-o em Configurações > Modelos.",
    );
    render(<NoteFailureBanner errorMessage={message} audioPreserved onRetry={() => undefined} />);
    expect(screen.getByRole("button", { name: /Tentar de novo/i })).toBeEnabled();
  } finally {
    applyInterfaceLocale("en");
  }
});
