#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// macOS keeps a privacy grant only while the running code still satisfies the
// designated requirement recorded with it. An Apple Development identity
// produces a requirement built from the bundle identifier and the certificate,
// so it survives rebuilds; an ad hoc signature pins the exact code hash and is
// invalidated by every rebuild. See docs/development.md.
export const DEV_SIGNING_IDENTITY_ENV = "CLOVY_DEV_SIGNING_IDENTITY";
export const AD_HOC_IDENTITY = "-";

const SECURITY = "/usr/bin/security";
const APPLE_DEVELOPMENT_PREFIX = "Apple Development: ";
const IDENTITY_LINE = /^\s*\d+\)\s+([0-9A-Fa-f]{40})\s+"(.*)"\s*$/;
const SHA1 = /^[0-9A-Fa-f]{40}$/;

export class DevSigningIdentityError extends Error {
  constructor(message) {
    super(message);
    this.name = "DevSigningIdentityError";
  }
}

/**
 * Selects the code signing identity shared by the development app bundle and
 * its helpers. Returns `{ identity, source }`, where `identity` is a
 * certificate SHA-1 accepted by `codesign --sign`, or `-` only when the
 * developer explicitly opted into ad hoc signing.
 */
export function resolveDevSigningIdentity({ env = process.env, run = spawnSync } = {}) {
  const configured = env[DEV_SIGNING_IDENTITY_ENV]?.trim();
  if (configured === AD_HOC_IDENTITY) {
    return { identity: AD_HOC_IDENTITY, source: "configured" };
  }

  const identities = validCodesigningIdentities(run);
  if (configured) {
    const matches = SHA1.test(configured)
      ? identities.filter((identity) => identity.hash === configured.toUpperCase())
      : identities.filter((identity) => identity.name === configured);
    if (matches.length !== 1) {
      throw new DevSigningIdentityError(
        `${DEV_SIGNING_IDENTITY_ENV} must name exactly one valid code signing identity; it matched ${matches.length}. ` +
          `Use a SHA-1 or exact name from \`security find-identity -v -p codesigning\`.`,
      );
    }
    return { identity: matches[0].hash, source: "configured" };
  }

  const appleDevelopment = identities.filter((identity) =>
    identity.name.startsWith(APPLE_DEVELOPMENT_PREFIX),
  );
  if (appleDevelopment.length === 1) {
    return { identity: appleDevelopment[0].hash, source: "discovered" };
  }
  throw new DevSigningIdentityError(
    appleDevelopment.length === 0
      ? `No valid Apple Development code signing identity was found. Set ${DEV_SIGNING_IDENTITY_ENV} to the SHA-1 of a valid code signing identity, ` +
          `or to "${AD_HOC_IDENTITY}" to accept that macOS asks for every permission again after each rebuild.`
      : `${appleDevelopment.length} valid Apple Development identities were found. Set ${DEV_SIGNING_IDENTITY_ENV} to the SHA-1 of the one to use ` +
          `(see \`security find-identity -v -p codesigning\`).`,
  );
}

function validCodesigningIdentities(run) {
  const result = run(SECURITY, ["find-identity", "-v", "-p", "codesigning"], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    const reason = `${result.stderr || result.stdout || ""}`.trim();
    throw new DevSigningIdentityError(
      `Could not list code signing identities${reason ? `: ${reason}` : ""}`,
    );
  }
  const identities = new Map();
  for (const line of `${result.stdout || ""}`.split("\n")) {
    const match = line.match(IDENTITY_LINE);
    if (match) identities.set(match[1].toUpperCase(), match[2]);
  }
  return [...identities].map(([hash, name]) => ({ hash, name }));
}

/**
 * True when `target` verifies strictly and carries the selected signature: a
 * leaf matching the certificate SHA-1, or an ad hoc signature for an explicit
 * `-`. Callers without a selection only check that the seal is valid.
 */
export function signatureSatisfies(target, signingIdentity, run = spawnSync) {
  if (signingIdentity !== AD_HOC_IDENTITY) {
    return (
      run(
        "/usr/bin/codesign",
        ["--verify", "--strict", `-R=certificate leaf = H"${signingIdentity}"`, target],
        { encoding: "utf8" },
      ).status === 0
    );
  }
  if (
    run("/usr/bin/codesign", ["--verify", "--strict", target], { encoding: "utf8" }).status !== 0
  ) {
    return false;
  }
  const details = run("/usr/bin/codesign", ["-dv", target], { encoding: "utf8" });
  return (
    details.status === 0 &&
    `${details.stdout || ""}\n${details.stderr || ""}`.split("\n").includes("Signature=adhoc")
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.stdout.write(`${resolveDevSigningIdentity().identity}\n`);
  } catch (error) {
    console.error(error instanceof DevSigningIdentityError ? error.message : error);
    process.exit(1);
  }
}
