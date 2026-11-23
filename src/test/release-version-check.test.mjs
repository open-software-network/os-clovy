import { describe, expect, it } from "vitest";
import { checkReleaseVersion } from "../../scripts/check-release-version.mjs";

function files(version) {
  return {
    tauriConf: `{\n  "productName": "Clovy",\n  "version": "${version}"\n}\n`,
    cargoToml: `[package]\nname = "clovy"\nversion = "${version}"\n`,
    cargoLock: `version = 4\n\n[[package]]\nname = "clovy"\nversion = "${version}"\n`,
    packageJson: `{\n  "name": "clovy",\n  "version": "${version}"\n}\n`,
  };
}

describe("checkReleaseVersion", () => {
  it("accepts a tag equal to the version in the four files", () => {
    expect(
      checkReleaseVersion({ tag: "v0.1.0", files: files("0.1.0"), releaseExists: false }),
    ).toEqual({ ok: true, version: "0.1.0" });
  });

  it("rejects a tag that differs from the version", () => {
    const result = checkReleaseVersion({
      tag: "v0.1.1",
      files: files("0.1.0"),
      releaseExists: false,
    });
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("v0.1.1");
    expect(result.reason).toContain("0.1.0");
  });

  it("rejects a tag that is not vX.Y.Z", () => {
    for (const tag of ["0.1.0", "v0.1", "v0.1.0-rc.1", "release-0.1.0"]) {
      expect(checkReleaseVersion({ tag, files: files("0.1.0"), releaseExists: false }).ok).toBe(
        false,
      );
    }
  });

  it("rejects version drift between the four files", () => {
    const drift = { ...files("0.1.0"), cargoToml: files("0.0.40").cargoToml };
    const result = checkReleaseVersion({ tag: "v0.1.0", files: drift, releaseExists: false });
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("drift");
  });

  it("rejects a tag whose release already exists", () => {
    const result = checkReleaseVersion({
      tag: "v0.1.0",
      files: files("0.1.0"),
      releaseExists: true,
    });
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("already exists");
  });

  // A dry run publishes nothing, so a patch branch can rehearse the release
  // of the version already out before the bump.
  it("lets a dry run rehearse a version whose release already exists", () => {
    expect(
      checkReleaseVersion({
        tag: "v0.1.0",
        files: files("0.1.0"),
        releaseExists: true,
        dryRun: true,
      }),
    ).toEqual({ ok: true, version: "0.1.0" });
  });
});
