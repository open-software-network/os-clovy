import { act, render } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { AgentBackground } from "../components/agent/AgentBackground";

vi.mock("asciify-engine/kit", () => {
  throw new Error("module unavailable");
});

it("leaves a plain background when the kit import rejects", async () => {
  localStorage.clear();
  const view = render(
    <>
      <AgentBackground />
      <div>Content</div>
    </>,
  );
  await act(async () => {});
  expect(view.getByText("Content")).toBeVisible();
  expect(view.container.querySelector("canvas")).toBeNull();
  view.unmount();
});
