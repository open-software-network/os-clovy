import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, rmdirSync, unlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

// Never steal a stale or unrecognised lock: a timed-out preparation leaves
// the other owner's files untouched and reports an explicit failure.
export async function acquirePreparationLock(cache, timeoutMs = 600000) {
  mkdirSync(cache, { recursive: true });
  const directory = path.join(cache, "preparation.lock");
  const owner = path.join(directory, "owner");
  const token = randomUUID();
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      mkdirSync(directory, { mode: 0o700 });
      break;
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      if (Date.now() >= deadline)
        throw new Error("Browser preparation lock is busy; no files were removed.");
      await delay(100);
    }
  }
  try {
    writeFileSync(owner, token, { flag: "wx", mode: 0o600 });
  } catch (error) {
    try {
      rmdirSync(directory);
    } catch {
      /* preserve unexpected contents */
    }
    throw error;
  }
  return () => {
    if (readFileSync(owner, "utf8") !== token)
      throw new Error("Browser preparation lock ownership changed.");
    unlinkSync(owner);
    rmdirSync(directory);
  };
}
