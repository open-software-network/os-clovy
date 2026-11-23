import { mkdtempSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { buildReleaseManifest, readReleaseAssets } from "../../scripts/release-manifest.mjs";

const MAC_SIG = "bWFjIHNpZ25hdHVyZQ==";
const WIN_SIG = "d2luZG93cyBzaWduYXR1cmU=";

function artifactDir(platforms = "all") {
  const dir = mkdtempSync(join(tmpdir(), "clovy-release-"));
  writeFileSync(join(dir, "Clovy_universal.dmg"), "dmg");
  writeFileSync(join(dir, "Clovy.app.tar.gz"), "tar");
  writeFileSync(join(dir, "Clovy.app.tar.gz.sig"), `${MAC_SIG}\n`);
  if (platforms === "all") {
    writeFileSync(join(dir, "Clovy_0.1.0_x64-setup.exe"), "exe");
    writeFileSync(join(dir, "Clovy_0.1.0_x64-setup.exe.sig"), WIN_SIG);
  }
  return dir;
}

describe("release manifest", () => {
  it.each([undefined, "all"])("lists all three platforms in mode %s", (platforms) => {
    const assets = readReleaseAssets(artifactDir(), platforms);
    const manifest = buildReleaseManifest({
      tag: "v0.1.0",
      repository: "guilhermexp/os-clovy",
      pubDate: "2026-10-22T12:00:00Z",
      assets,
      platforms,
    });

    const base = "https://github.com/guilhermexp/os-clovy/releases/download/v0.1.0";
    const mac = { signature: MAC_SIG, url: `${base}/Clovy.app.tar.gz` };
    expect(manifest).toEqual({
      version: "0.1.0",
      pub_date: "2026-10-22T12:00:00Z",
      platforms: {
        "darwin-aarch64": mac,
        "darwin-x86_64": mac,
        "windows-x86_64": {
          signature: WIN_SIG,
          url: `${base}/Clovy_0.1.0_x64-setup.exe`,
        },
      },
    });
  });

  it("lists the required release artifact files", () => {
    const assets = readReleaseAssets(artifactDir());
    expect(assets.files.map((file) => file.split("/").pop()).sort()).toEqual([
      "Clovy.app.tar.gz",
      "Clovy.app.tar.gz.sig",
      "Clovy_0.1.0_x64-setup.exe",
      "Clovy_0.1.0_x64-setup.exe.sig",
      "Clovy_universal.dmg",
    ]);
  });

  it("fails when a package or its signature is missing", () => {
    const dir = mkdtempSync(join(tmpdir(), "clovy-release-"));
    writeFileSync(join(dir, "Clovy.app.tar.gz"), "tar");
    expect(() => readReleaseAssets(dir)).toThrow(/Clovy\.app\.tar\.gz\.sig/);
  });

  it("publishes only the two Mac architectures from Mac artifacts", () => {
    const assets = readReleaseAssets(artifactDir("macos"), "macos");
    const manifest = buildReleaseManifest({
      tag: "v0.1.6",
      repository: "guilhermexp/os-clovy",
      pubDate: "2026-11-06T12:00:00Z",
      assets,
      platforms: "macos",
    });
    const mac = {
      signature: MAC_SIG,
      url: "https://github.com/guilhermexp/os-clovy/releases/download/v0.1.6/Clovy.app.tar.gz",
    };
    expect(manifest).toEqual({
      version: "0.1.6",
      pub_date: "2026-11-06T12:00:00Z",
      platforms: { "darwin-aarch64": mac, "darwin-x86_64": mac },
    });
    expect(assets.files.map((file) => file.split("/").pop()).sort()).toEqual([
      "Clovy.app.tar.gz",
      "Clovy.app.tar.gz.sig",
      "Clovy_universal.dmg",
    ]);
  });

  it.each([undefined, "all"])("requires Windows artifacts in mode %s", (platforms) => {
    expect(() => readReleaseAssets(artifactDir("macos"), platforms)).toThrow(/Windows/);
  });

  it("requires the Windows signature in all mode", () => {
    const dir = artifactDir();
    unlinkSync(join(dir, "Clovy_0.1.0_x64-setup.exe.sig"));
    expect(() => readReleaseAssets(dir, "all")).toThrow(/Clovy_0.1.0_x64-setup.exe.sig/);
  });

  it.each([
    "Clovy.app.tar.gz",
    "Clovy.app.tar.gz.sig",
    "Clovy_universal.dmg",
  ])("requires %s in Mac mode", (name) => {
    const dir = artifactDir("macos");
    unlinkSync(join(dir, name));
    expect(() => readReleaseAssets(dir, "macos")).toThrow(name);
  });

  it("omits a Windows artifact even when one is present in Mac mode", () => {
    const assets = readReleaseAssets(artifactDir(), "macos");
    expect(assets.windows).toBeUndefined();
    expect(assets.files).toHaveLength(3);
  });

  it.each(["windows", "ALL", "", "macos,windows"])("rejects invalid mode %s", (platforms) => {
    expect(() => readReleaseAssets(artifactDir(), platforms)).toThrow(/platforms/i);
    expect(() =>
      buildReleaseManifest({
        tag: "v0.1.6",
        repository: "guilhermexp/os-clovy",
        pubDate: "2026-11-06T12:00:00Z",
        assets: readReleaseAssets(artifactDir()),
        platforms,
      }),
    ).toThrow(/platforms/i);
  });
});
