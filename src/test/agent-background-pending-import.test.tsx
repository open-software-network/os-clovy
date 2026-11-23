import { act, render } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { AgentBackground } from "../components/agent/AgentBackground";

const kit = vi.hoisted(() => {
  let release = () => {};
  const ready = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { ready, release, background: vi.fn() };
});
vi.mock("asciify-engine/kit", async () => {
  await kit.ready;
  return { background: kit.background, drift: { id: "drift" } };
});

it("does not initialize after unmount while the import is pending", async () => {
  localStorage.clear();
  const view = render(
    <>
      <AgentBackground />
      <div>Content</div>
    </>,
  );
  view.unmount();
  await act(async () => {
    kit.release();
  });
  await import("asciify-engine/kit");
  await act(async () => {});
  expect(kit.background).not.toHaveBeenCalled();
});
