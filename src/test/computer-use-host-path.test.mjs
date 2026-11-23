import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { resolveComputerUseHost } from "../../scripts/computer-use-host-path.mjs";

describe("Computer use host discovery", () => {
  it("selects native or explicitly targeted output and respects the host override", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "clovy-host-discovery-"));
    try {
      const native = path.join(dir, "debug", "clovy.exe");
      const target = "x86_64-pc-windows-msvc";
      const targeted = path.join(dir, target, "debug", "clovy.exe");
      mkdirSync(path.dirname(native), { recursive: true });
      mkdirSync(path.dirname(targeted), { recursive: true });
      writeFileSync(native, "native fixture");
      writeFileSync(targeted, "targeted fixture");
      const options = { targetDir: dir, profile: "debug", platform: "win32" };
      expect(resolveComputerUseHost(options)).toBe(native);
      expect(resolveComputerUseHost({ ...options, targetTriple: target })).toBe(targeted);
      expect(resolveComputerUseHost({ ...options, targetTriple: target, host: native })).toBe(
        native,
      );
      expect(() =>
        resolveComputerUseHost({ ...options, targetTriple: "aarch64-pc-windows-msvc" }),
      ).toThrow();
      expect(() => resolveComputerUseHost({ ...options, profile: "release" })).toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
