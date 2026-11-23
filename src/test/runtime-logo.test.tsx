import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { RuntimeLogo, runtimeBrand } from "../components/brand/RuntimeLogo";

describe("RuntimeLogo", () => {
  it.each([
    ["claude", "claudecode", "Claude Code"],
    ["codex", "codex", "Codex"],
    ["copilot", "githubcopilot", "GitHub Copilot"],
    ["cursor-agent", "cursor", "Cursor"],
    ["agy", "antigravity", "Antigravity"],
    ["pi", "pi", "Pi"],
    ["openrouter", "openrouter", "OpenRouter"],
  ])("draws the %s brand mark", (id, brand, label) => {
    expect(runtimeBrand(id)).toBe(brand);
    render(<RuntimeLogo id={id} />);

    const mark = screen.getByRole("img", { name: label });
    expect(mark.tagName.toLowerCase()).toBe("svg");
    expect(mark).toHaveAttribute("data-brand", brand);
    expect(mark).toHaveAttribute("viewBox", "0 0 24 24");
    expect(mark.querySelector("path")).not.toBeNull();
    expect(mark).toHaveAttribute("fill", "currentColor");
  });

  it.each([
    ["claude_code", "claudecode"],
    ["copilot_cli", "githubcopilot"],
    ["copilot_vscode", "githubcopilot"],
    ["cursor", "cursor"],
    ["cursor_cli", "cursor"],
    ["antigravity", "antigravity"],
  ])("maps the coding-agent source %s to its brand", (id, brand) => {
    expect(runtimeBrand(id)).toBe(brand);
  });

  it("falls back to a two-letter monogram for a brand without an SVG", () => {
    expect(runtimeBrand("lab-server")).toBeNull();
    const { container } = render(<RuntimeLogo id="lab-server" name="Lab server" size={16} />);

    const monogram = container.querySelector('[data-brand="monogram"]');
    expect(monogram).not.toBeNull();
    expect(monogram).toHaveTextContent("LS");
    expect(monogram).toHaveStyle({ width: "16px", height: "16px" });
  });

  it("keeps the requested size for brand marks", () => {
    render(<RuntimeLogo id="codex" size={14} />);
    const mark = screen.getByRole("img", { name: "Codex" });
    expect(mark).toHaveAttribute("width", "14");
    expect(mark).toHaveAttribute("height", "14");
  });
});
