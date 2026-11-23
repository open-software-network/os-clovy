import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(path, "utf8");

describe("CLI-only desktop packaging", () => {
  it("does not build or bundle a native agent sidecar on either platform", () => {
    for (const path of [
      "package.json",
      "pnpm-workspace.yaml",
      "src-tauri/build.rs",
      "src-tauri/tauri.conf.json",
      "src-tauri/tauri.macos.conf.json",
      "src-tauri/tauri.windows.conf.json",
      "scripts/tauri-before-dev.mjs",
      "scripts/build-macos-release.sh",
      "scripts/build-windows.ps1",
      "scripts/qualify-windows-build.ps1",
    ]) {
      expect(read(path), path).not.toMatch(/agent-runtime|AGENT_RUNTIME|AgentRuntimeEntitlements/);
    }
    for (const path of [
      "agent-runtime/package.json",
      "scripts/build-agent-runtime.mjs",
      "scripts/benchmark-agent-runtime.mjs",
      "src-tauri/AgentRuntimeEntitlements.plist",
    ]) {
      expect(existsSync(path), path).toBe(false);
    }
  });

  it("rejects stale sidecars in the final Windows installer", () => {
    const verifier = read("scripts/verify-windows-artifacts.ps1");
    expect(verifier).toContain("clovy-agent-runtime");
    expect(verifier).toContain("Retired agent runtime payload found");
    expect(verifier).not.toContain('Require-One "clovy-agent-runtime.exe"');
  });
});
