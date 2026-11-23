import { linkSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  allocateEngineBuildSource,
  helperBuildInputsMatch,
  sha256,
  verifyEngineArtifact,
  verifyEngineSbom,
} from "../../scripts/agent-desktop-artifact.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const pin = JSON.parse(readFileSync(join(root, "src-tauri/agent-desktop-pin.json"), "utf8"));
const directories = [];
afterEach(() => {
  for (const dir of directories.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function artifact() {
  const directory = mkdtempSync(join(tmpdir(), "clovy-engine-artifact-"));
  directories.push(directory);
  // A real local executable exercises artifact-byte validation without building
  // or executing an engine or downloading anything during a unit test.
  linkSync(process.execPath, join(directory, "agent-desktop"));
  writeFileSync(
    join(directory, "LICENSE"),
    readFileSync(join(root, "src-tauri/agent-desktop/LICENSE")),
  );
  writeFileSync(
    join(directory, "source-review.md"),
    readFileSync(join(root, "src-tauri/agent-desktop/source-review.md")),
  );
  const target = "x86_64-unknown-linux-gnu";
  const manifest = {
    ...pin,
    target,
    executable: "agent-desktop",
    executableSha256: sha256(join(directory, "agent-desktop")),
  };
  const save = () =>
    writeFileSync(join(directory, "agent-desktop-manifest.json"), JSON.stringify(manifest));
  save();
  return { directory, target, manifest, save };
}

describe("bundled desktop engine artifact verification", () => {
  it("rejects a packaged SBOM whose patch provenance differs from the pin", () => {
    const dir = mkdtempSync(join(tmpdir(), "clovy-sbom-proof-"));
    directories.push(dir);
    const sbom = JSON.parse(
      readFileSync(join(root, "src-tauri/agent-desktop/sbom.spdx.json"), "utf8"),
    );
    const file = join(dir, "agent-desktop.spdx.json");
    writeFileSync(file, JSON.stringify(sbom));
    expect(verifyEngineSbom(file, pin).packages[0].versionInfo).toBe(pin.version);
    sbom.packages[0].sourceInfo = "another patch";
    writeFileSync(file, JSON.stringify(sbom));
    expect(() => verifyEngineSbom(file, pin)).toThrow(/provenance mismatch/);
    sbom.packages[0].sourceInfo = `reviewed patch SHA-256 ${pin.patchSha256}.`;
    sbom.documentNamespace += "-different";
    writeFileSync(file, JSON.stringify(sbom));
    expect(() => verifyEngineSbom(file, pin)).toThrow(/provenance mismatch/);
  });
  it("preserves earlier checkout generations and refuses an occupied explicit directory", () => {
    const root = mkdtempSync(join(tmpdir(), "clovy-build-generations-"));
    directories.push(root);
    const first = allocateEngineBuildSource(root, "source", undefined);
    writeFileSync(join(first, "evidence"), "preserve");
    const second = allocateEngineBuildSource(root, "source", undefined);
    const third = allocateEngineBuildSource(root, "source", undefined);
    expect(new Set([first, second, third]).size).toBe(3);
    expect(readFileSync(join(first, "evidence"), "utf8")).toBe("preserve");
    expect(() => allocateEngineBuildSource(root, "source", first)).toThrow(/Refusing to overwrite/);
    expect(readFileSync(join(first, "evidence"), "utf8")).toBe("preserve");
  });
  it("accepts byte-identical executable and license with the reviewed source contract", () => {
    const { directory, target, manifest } = artifact();
    expect(verifyEngineArtifact(directory, pin, target).executableSha256).toBe(
      manifest.executableSha256,
    );
  });
  it("rejects older or different runtime source generations with identical artifact bytes", () => {
    const { directory, target, manifest, save } = artifact();
    delete manifest.ownedRuntimeSourceSha256;
    save();
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow();
    manifest.ownedRuntimeSourceSha256 = "different-generation";
    save();
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow();
    manifest.ownedRuntimeSourceSha256 = pin.ownedRuntimeSourceSha256;
    save();
    expect(() =>
      verifyEngineArtifact(directory, { ...pin, ownedRuntimeSourceSha256: "" }, target),
    ).toThrow();
  });
  it.each([
    "sourceCommit",
    "protocolVersion",
    "patchSha256",
    "cargoLockSha256",
    "licenseSha256",
    "sourceReviewSha256",
  ])("refuses a changed %s before execution", (field) => {
    const { directory, target, manifest, save } = artifact();
    manifest[field] = "unreviewed";
    save();
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow(`${field} mismatch`);
  });
  it("rebuilds the helper when a valid new binary generation retains the same source", () => {
    const { directory, target, manifest, save } = artifact();
    const expected = {
      profile: "debug",
      appIdentifier: "dev.guilhermexp.clovy",
      sourceSha256: sha256(join(root, "src-tauri/src/computer_use_driver.rs")),
      engineSha256: manifest.executableSha256,
    };
    const helperStamp = { clovyBuild: { ...expected } };
    expect(helperBuildInputsMatch(helperStamp, expected)).toBe(true);
    const executable = join(directory, "agent-desktop");
    unlinkSync(executable);
    writeFileSync(executable, "new executable generation with unchanged source");
    manifest.executableSha256 = sha256(executable);
    save();
    const next = verifyEngineArtifact(directory, pin, target);
    expect(next.sourceCommit).toBe(pin.sourceCommit);
    expect(
      helperBuildInputsMatch(helperStamp, { ...expected, engineSha256: next.executableSha256 }),
    ).toBe(false);
  });
  it("refuses a different executable generation under unchanged source and manifest", () => {
    const { directory, target } = artifact();
    const executable = join(directory, "agent-desktop");
    unlinkSync(executable); // The fixture starts as a hard link to Node; never edit that inode.
    writeFileSync(executable, "different executable generation");
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow(
      "executable digest mismatch",
    );
  });
  it("refuses bytes that differ from the signed manifest receipt", () => {
    const { directory, target, manifest, save } = artifact();
    manifest.executableSha256 = "0".repeat(64);
    save();
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow(
      "executable digest mismatch",
    );
  });
  it("refuses another architecture and a path outside the engine directory", () => {
    const { directory, target, manifest, save } = artifact();
    expect(() => verifyEngineArtifact(directory, pin, "aarch64-unknown-linux-gnu")).toThrow(
      "target mismatch",
    );
    manifest.executable = "../agent-desktop";
    save();
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow("executable path mismatch");
  });
  it("refuses changed source qualification bytes", () => {
    const { directory, target } = artifact();
    writeFileSync(join(directory, "source-review.md"), "Changed provenance");
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow(
      "source qualification digest mismatch",
    );
  });
  it("refuses changed license bytes", () => {
    const { directory, target } = artifact();
    writeFileSync(join(directory, "LICENSE"), "Different license");
    expect(() => verifyEngineArtifact(directory, pin, target)).toThrow("license digest mismatch");
  });
});
