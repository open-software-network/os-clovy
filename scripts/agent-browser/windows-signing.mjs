import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

export function windowsSigningIdentity(env) {
  const thumbprint = env.WINDOWS_CERTIFICATE_THUMBPRINT?.trim();
  if (thumbprint) return `store-sha1:${thumbprint.toUpperCase()}`;
  const certificate = env.WINDOWS_CERTIFICATE_PATH?.trim();
  const password = env.WINDOWS_CERTIFICATE_PASSWORD?.trim();
  if (!!certificate !== !!password)
    throw new Error(
      "Both WINDOWS_CERTIFICATE_PATH and WINDOWS_CERTIFICATE_PASSWORD are required for PFX signing.",
    );
  return certificate
    ? `pfx-sha256:${createHash("sha256").update(readFileSync(certificate)).digest("hex")}`
    : null;
}
