import { createHash } from "node:crypto";
import path from "node:path";

export function computerUseBundleIdentifier({ baseIdentifier, profile, worktreeRoot }) {
  if (profile === "release") return baseIdentifier;
  if (profile !== "debug") {
    throw new Error(`Unsupported Computer use helper profile: ${profile}`);
  }
  return `${baseIdentifier}.dev.${computerUseWorktreeKey(worktreeRoot)}`;
}

export function computerUseWorktreeKey(worktreeRoot) {
  const normalized = path.resolve(worktreeRoot);
  const digest = createHash("sha256").update(normalized).digest("hex").slice(0, 12);
  return `w${digest}`;
}
