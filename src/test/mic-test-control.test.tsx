import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MicTestControl } from "../components/settings/MicTestControl";

afterEach(() => {
  vi.useRealTimers();
});

describe("MicTestControl", () => {
  it("runs the recording clock without re-rendering its parent", () => {
    vi.useFakeTimers({ now: 10_000 });
    let parentRenders = 0;
    function Parent() {
      parentRenders++;
      return (
        <MicTestControl
          state="recording"
          level={0.5}
          startedAt={10_000}
          playing={false}
          durationSeconds={5}
          onStart={() => {}}
          onStartOver={() => {}}
          onPlaybackError={() => {}}
          onPlayingChange={() => {}}
        />
      );
    }
    render(<Parent />);
    expect(screen.getByText("00:00 / 00:05")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(2_000);
    });

    expect(screen.getByText("00:02 / 00:05")).toBeInTheDocument();
    expect(parentRenders).toBe(1);
  });
});
