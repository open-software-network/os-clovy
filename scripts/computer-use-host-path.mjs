import { existsSync } from "node:fs";
import path from "node:path";

export function resolveComputerUseHost({ host, targetDir, profile, platform, targetTriple }) {
  const selected = host
    ? path.resolve(host)
    : path.join(
        targetDir,
        ...(targetTriple ? [targetTriple] : []),
        profile,
        platform === "win32" ? "clovy.exe" : "clovy",
      );
  if (!existsSync(selected)) {
    throw new Error(
      `Computer use self-test host is missing: ${selected}. Build Clovy first or pass --host.`,
    );
  }
  return selected;
}
