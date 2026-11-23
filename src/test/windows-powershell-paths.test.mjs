import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

const powershell = ["pwsh", "powershell.exe"].find(
  (command) =>
    spawnSync(command, ["-NoProfile", "-NonInteractive", "-?"], { encoding: "utf8" }).status === 0,
);

describe.skipIf(!powershell)("real PowerShell file-parameter consumers", () => {
  for (const script of ["windows-authenticode.ps1", "windows-sign.ps1"]) {
    it(`${script} binds literal paths with spaces/apostrophes without executing path text`, () => {
      const root = mkdtempSync(path.join(tmpdir(), "clovy-powershell-"));
      try {
        const marker = path.join(root, "injected");
        const target = path.join(root, `missing ' ; New-Item injected ; # space.exe`);
        const result = spawnSync(
          powershell,
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            path.resolve("scripts", script),
            "-TargetPath",
            target,
          ],
          { cwd: root, encoding: "utf8" },
        );
        expect(result.status).not.toBe(0);
        expect(`${result.stdout}${result.stderr}`).toMatch(
          /TargetPath does not point to a readable file/,
        );
        expect(existsSync(marker)).toBe(false);
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    });
  }
});
