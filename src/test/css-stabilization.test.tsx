import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Checkbox } from "../components/ui/Checkbox";
import { EmptyState } from "../components/ui/EmptyState";
import appCss from "../styles/app.css?raw";

describe("CSS stabilization state selectors", () => {
  it("does not use :has() in the application stylesheet", () => {
    expect(appCss).not.toContain(":has(");
  });

  it("uses composited properties for waveform and progress animation", () => {
    expect(appCss).toContain("transition: transform 18ms linear;");
    expect(appCss).toContain("transition: clip-path 18ms linear;");
    expect(appCss).not.toMatch(/transition:[ \t]*(?:height|width)\b/);
  });

  it("keeps the intrinsic accordion transitions that reflow surrounding content", () => {
    expect(appCss.match(/transition:\s*grid-template-rows\b/g)).toHaveLength(2);
  });

  it("marks shared component state explicitly", () => {
    const { container, rerender } = render(
      <>
        <Checkbox checked disabled onChange={() => undefined} />
        <EmptyState title="Nothing here" footer={<span>Shortcut</span>} />
      </>,
    );

    expect(container.querySelector(".checkbox-control")).toHaveClass(
      "checkbox-control-checked",
      "checkbox-control-disabled",
    );
    expect(container.querySelector(".empty-state")).toHaveClass("empty-state-with-footer");

    rerender(
      <>
        <Checkbox checked={false} onChange={() => undefined} />
        <EmptyState title="Nothing here" />
      </>,
    );

    expect(container.querySelector(".checkbox-control")).not.toHaveClass(
      "checkbox-control-checked",
      "checkbox-control-disabled",
    );
    expect(container.querySelector(".empty-state")).not.toHaveClass("empty-state-with-footer");
  });
});
