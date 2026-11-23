import { describe, expect, it } from "vitest";
import {
  humanizeToolName,
  toolActivityLabel,
  toolActivitySentence,
} from "../lib/agent-tool-labels";

describe("toolActivityLabel", () => {
  it("replaces generic terminal labels with the command activity", () => {
    expect(toolActivityLabel("terminal")).toBe("Running command");
    expect(
      toolActivityLabel("terminal", {
        command: "curl https://example.com/docs",
      }),
    ).toBe("Browsing");
    expect(toolActivityLabel("shell", { command: "rg -n Terminal src" })).toBe("Searching files");
  });

  it("labels common web and file tools by intent", () => {
    expect(toolActivityLabel("web.run", { search_query: [{ q: "Clovy status" }] })).toBe(
      "Searching web",
    );
    expect(toolActivityLabel("fetch_url", { url: "https://example.com" })).toBe("Browsing");
    expect(toolActivityLabel("read_file", { path: "src/App.tsx" })).toBe("Reading files");
    expect(toolActivityLabel("write_file", { path: "src/App.tsx" })).toBe("Editing files");
  });

  it("labels the june_video MCP tools as video work, not a raw tool name", () => {
    // Regression: `mcp__june_video__generate_video` used to humanize to the ugly
    // "Mcp june video generate video".
    expect(toolActivityLabel("mcp__june_video__generate_video")).toBe("Working with video");
    expect(toolActivityLabel("june_video.generate_video")).toBe("Working with video");
    // animate_image is image-to-video — video work, not "Working with images".
    expect(toolActivityLabel("june_video.animate_image")).toBe("Working with video");
  });

  it("labels Clovy's private desktop tool as Computer use", () => {
    expect(toolActivityLabel("mcp__june_computer_use__computer_use")).toBe("Computer use");
    expect(toolActivityLabel("june_computer_use.computer_use")).toBe("Computer use");
    expect(toolActivityLabel("computer_user_lookup")).toBe("Computer user lookup");
  });

  it("keeps an understandable fallback for unknown tools", () => {
    expect(humanizeToolName("custom_deploy_tool")).toBe("Custom deploy tool");
    expect(toolActivityLabel("custom_deploy_tool")).toBe("Custom deploy tool");
  });

  it("composes activity labels as standalone status sentences", () => {
    expect(toolActivitySentence("read_file")).toBe("Reading files.");
    expect(toolActivitySentence("gh")).toBe("Using GitHub.");
    expect(toolActivitySentence(undefined)).toBe("Using a tool.");
  });
});
