import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// PowerShell does not run on the macOS gate; the Windows release job runs both
// scripts for real (release.yml). These pin the contract they must keep.
const build = readFileSync("scripts/build-windows.ps1", "utf8");
const verify = readFileSync("scripts/verify-windows-artifacts.ps1", "utf8");

describe("Windows updater artifacts", () => {
  it("builds the signed updater package only when the updater key exists", () => {
    expect(build).toMatch(
      /\$hasUpdaterKey = -not \[string\]::IsNullOrWhiteSpace\(\$env:TAURI_SIGNING_PRIVATE_KEY\)/,
    );
    expect(build).toContain('"src-tauri/tauri.release.conf.json"');
    expect(build).toContain('"src-tauri/tauri.windows.conf.json"');
    expect(build).toContain('"$($installers[0].FullName).sig"');
    // The staged installer explicitly retains the updater signature independently of Authenticode.
    expect(build).toContain("& pnpm tauri signer sign $installers[0].FullName");
  });

  it("requires the installer's minisign signature when the updater key exists", () => {
    expect(verify).toMatch(
      /\$hasUpdaterKey = -not \[string\]::IsNullOrWhiteSpace\(\$env:TAURI_SIGNING_PRIVATE_KEY\)/,
    );
    expect(verify).toContain('$signaturePath = "$($installer.FullName).sig"');
    expect(verify).toContain("untrusted comment:");
    expect(verify).toContain("The updater signature is missing");
  });
});
