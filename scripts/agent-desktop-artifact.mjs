import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export function allocateEngineBuildSource(buildRoot, prefix, override) {
  if (override) {
    const destination = resolve(override);
    mkdirSync(dirname(destination), { recursive: true });
    try {
      mkdirSync(destination);
    } catch (error) {
      if (error.code === "EEXIST")
        throw new Error(`Refusing to overwrite engine build checkout: ${destination}`);
      throw error;
    }
    return destination;
  }
  mkdirSync(buildRoot, { recursive: true });
  return mkdtempSync(join(buildRoot, `${prefix}-`));
}

export function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function verifyEngineSbom(file, pin) {
  const sbom = JSON.parse(
    Buffer.isBuffer(file) ? file.toString("utf8") : readFileSync(file, "utf8"),
  );
  const entry = sbom.packages?.find((item) => item.name === "agent-desktop");
  const source = entry?.externalRefs?.find(
    (item) => item.referenceType === "purl",
  )?.referenceLocator;
  if (
    sbom.documentNamespace !==
      `https://github.com/guilhermexp/os-clovy/sbom/agent-desktop/${pin.sourceCommit}/${pin.patchSha256}` ||
    entry?.versionInfo !== pin.version ||
    entry?.licenseDeclared !== "Apache-2.0" ||
    source !== `pkg:github/vercel-labs/agent-desktop@${pin.sourceCommit}` ||
    !entry?.sourceInfo?.includes(`reviewed patch SHA-256 ${pin.patchSha256}.`)
  )
    throw new Error("Engine SBOM provenance mismatch");
  return sbom;
}

// Verify bytes, not a successful --version invocation from an untrusted file.
export function verifyEngineArtifact(directory, pin, target) {
  const manifest = JSON.parse(readFileSync(join(directory, "agent-desktop-manifest.json"), "utf8"));
  for (const field of [
    "source",
    "sourceCommit",
    "version",
    "protocolVersion",
    "cargoLockSha256",
    "license",
    "licenseSha256",
    "patchSha256",
    "ownedRuntimeSourceSha256",
    "sourceReviewSha256",
  ]) {
    if (typeof pin[field] !== "string" || !pin[field] || manifest[field] !== pin[field])
      throw new Error(`Engine manifest ${field} mismatch`);
  }
  if (manifest.target !== target) throw new Error("Engine manifest target mismatch");
  const executable = target.includes("windows") ? "agent-desktop.exe" : "agent-desktop";
  if (manifest.executable !== executable) throw new Error("Engine executable path mismatch");
  if (sha256(join(directory, executable)) !== manifest.executableSha256) {
    throw new Error("Engine executable digest mismatch");
  }
  if (sha256(join(directory, "LICENSE")) !== pin.licenseSha256)
    throw new Error("Engine license digest mismatch");
  if (sha256(join(directory, "source-review.md")) !== pin.sourceReviewSha256)
    throw new Error("Engine source qualification digest mismatch");
  return manifest;
}

// A valid source pin can produce several signed binary generations. The helper
// cache must bind the exact generation compiled into its executable.
export function helperBuildInputsMatch(stamp, expected) {
  return ["profile", "appIdentifier", "sourceSha256", "engineSha256"].every(
    (field) => typeof expected[field] === "string" && stamp.clovyBuild?.[field] === expected[field],
  );
}
