import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { devAppIdentityForBranch } from "../../scripts/dev-app-identity.mjs";
import { clovyMarkPalette } from "../../scripts/generate-icons.mjs";

const iconTemplate = readFileSync(
  resolve(process.cwd(), "src-tauri/icons/themed/_src/icon.template.svg"),
  "utf8",
);
describe("development app identity", () => {
  it("keeps the visible name stable on a Codex issue branch", () => {
    expect(devAppIdentityForBranch("codex/jun-278-computer-use")).toEqual({
      productName: "Clovy",
      identifier: "dev.guilhermexp.clovy.codex.jun278",
    });
  });

  it("normalizes the internal issue identity without exposing it in the name", () => {
    expect(devAppIdentityForBranch("codex/fix-JUN-00278-permissions")).toEqual({
      productName: "Clovy",
      identifier: "dev.guilhermexp.clovy.codex.jun00278",
    });
  });

  it("isolates Claude worktrees under the same visible name", () => {
    expect(devAppIdentityForBranch("claude/jun-278-computer-use")).toEqual({
      productName: "Clovy",
      identifier: "dev.guilhermexp.clovy.claude.jun278",
    });
  });

  it.each([
    "main",
    "codex/refactor-dev-launch",
    "jakub/jun-278-integration",
    "",
  ])("keeps the normal identity for %s", (branch) => {
    expect(devAppIdentityForBranch(branch)).toEqual({
      productName: "Clovy",
      identifier: "dev.guilhermexp.clovy",
    });
  });

  it("lets an explicit identifier isolate a run from any branch", () => {
    expect(
      devAppIdentityForBranch("codex/jun-278-computer-use", {
        identifierOverride: " dev.guilhermexp.clovy.permproof ",
      }),
    ).toEqual({ productName: "Clovy", identifier: "dev.guilhermexp.clovy.permproof" });
    expect(() =>
      devAppIdentityForBranch("main", { identifierOverride: "../dev.guilhermexp.clovy" }),
    ).toThrow("Invalid CLOVY_DEV_APP_IDENTIFIER: ../dev.guilhermexp.clovy");
  });
});

describe("Clovy themed icon generation", () => {
  it("keeps the canonical lime material for Sage", () => {
    expect(clovyMarkPalette("sage", "#3f812f")).toEqual({
      top: "#F0FF92",
      high: "#E2FF6D",
      mid: "#D7FF54",
      bottom: "#B0FA65",
      strokeTop: "#F6FFC4",
      strokeBottom: "#54D55F",
    });
  });

  it("gives every other preset a distinct luminous Clovy material", () => {
    const palettes = [
      clovyMarkPalette("rose", "#a5655c"),
      clovyMarkPalette("clay", "#b5551f"),
      clovyMarkPalette("ocean", "#3d7b9a"),
      clovyMarkPalette("plum", "#965d84"),
    ];

    expect(new Set(palettes.map((palette) => palette.bottom)).size).toBe(4);
    for (const palette of palettes) {
      expect(Object.values(palette).every((color) => /^#[0-9a-f]{6}$/i.test(color))).toBe(true);
      expect(palette.bottom).not.toBe("#B0FA65");
    }
  });

  it("routes every mark material stop through the generator", () => {
    for (const placeholder of [
      "{{MARK_TOP}}",
      "{{MARK_HIGH}}",
      "{{MARK_MID}}",
      "{{MARK_BOTTOM}}",
      "{{MARK_STROKE_TOP}}",
      "{{MARK_STROKE_BOTTOM}}",
    ]) {
      expect(iconTemplate).toContain(placeholder);
    }
  });
});

describe("Vitest resource limits", () => {
  it("keeps every frontend test command in a bounded worker-thread pool", () => {
    const configs = [readFileSync(resolve("vite.config.ts"), "utf8")];

    for (const config of configs) {
      const testConfig = config.slice(config.indexOf("  test: {"));
      expect(testConfig).toContain('pool: "threads"');
      expect(testConfig).toContain("fileParallelism: false");
      expect(testConfig).toContain("maxWorkers: 1");
    }
  });
});
