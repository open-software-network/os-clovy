import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { NoteRecoveryPrompt } from "../components/recorder/NoteRecoveryPrompt";
import type { RecoverableRecordingDto } from "../lib/tauri";

const recovery: RecoverableRecordingDto = {
  sessionId: "session-1",
  noteId: "note-1",
  startedAt: "2026-05-19T10:00:00Z",
  partialPathPresent: true,
  finalPathPresent: false,
  bytesFound: 4096,
};

function rectList(lineTops: number[]): DOMRectList {
  const rects = lineTops.map((top) => ({ height: 24, top, width: 200 }));
  return Object.assign(rects, {
    item: (index: number) => rects[index] ?? null,
  }) as unknown as DOMRectList;
}

describe("NoteRecoveryPrompt", () => {
  it("surfaces recoverable recordings with recover and discard actions", async () => {
    const user = userEvent.setup();
    const onRecover = vi.fn();
    const onDiscard = vi.fn();
    render(<NoteRecoveryPrompt recovery={recovery} onRecover={onRecover} onDiscard={onDiscard} />);

    expect(screen.getByLabelText("Recoverable recording")).not.toHaveClass(
      "note-recovery-prompt-blocked",
    );
    expect(screen.getByText(/recording was interrupted/i)).toBeInTheDocument();
    expect(screen.getByText(/4\.0 KB/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Recover" }));
    await user.click(screen.getByRole("button", { name: "Discard" }));

    expect(onRecover).toHaveBeenCalledWith("session-1");
    expect(onDiscard).toHaveBeenCalledWith("session-1");
  });

  it("centers only while the unblocked body occupies one rendered line", () => {
    let lineTops = [10];
    let resizeCallback: ResizeObserverCallback | undefined;
    const disconnect = vi.fn();
    vi.spyOn(Range.prototype, "getClientRects").mockImplementation(() => rectList(lineTops));
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: ResizeObserverCallback) {
          resizeCallback = callback;
        }
        observe() {}
        disconnect() {
          disconnect();
        }
      },
    );

    const { unmount } = render(
      <NoteRecoveryPrompt recovery={recovery} onRecover={vi.fn()} onDiscard={vi.fn()} />,
    );

    const prompt = screen.getByLabelText("Recoverable recording");
    expect(prompt).toHaveClass("note-recovery-prompt-single-line");

    lineTops = [10, 34];
    act(() => resizeCallback?.([], {} as ResizeObserver));
    expect(prompt).not.toHaveClass("note-recovery-prompt-single-line");

    lineTops = [10];
    act(() => resizeCallback?.([], {} as ResizeObserver));
    expect(prompt).toHaveClass("note-recovery-prompt-single-line");

    unmount();
    expect(disconnect).toHaveBeenCalledOnce();
  });
});
